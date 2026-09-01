#![deny(unsafe_code)]

//! Nimino instance administration CLI.
//!
//! # Member management (NIP-43)
//!
//! ## Why only kind:13534 (membership list), not kind:8000/8001 (deltas)
//!
//! CLI intentionally does not emit kind 8000/8001 deltas —
//! The CLI writes the authoritative kind:13534 roster snapshot. Live cluster
//! propagation belongs to the Nim sync domain; this separate process does not
//! pretend that a process-local channel can notify the relay.
//!
//! ## Same-second domination guard
//!
//! The `custom_created_at = max(now, newest_existing_13534 + 1s)` bump defeats
//! same-second domination for serial invocations; it does NOT serialize
//! concurrent CLI processes — two near-simultaneous adds can read the same
//! newest timestamp and collide on the bumped second. run.sh serialization is
//! the guard against parallel adds (e.g. `xargs -P`).

mod deletions;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nimino_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
use nimino_core::tenant::{relay_url_authority, TenantContext};
use nimino_db::{Db, DbConfig};
use nostr::{EventBuilder, Keys, Kind, Tag};

#[derive(Parser)]
#[command(name = "nimino-admin", about = "Nimino instance administration")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a pubkey to the relay membership list.
    ///
    /// Accepts a bech32 npub or 64-char hex pubkey. After inserting the DB row,
    /// stores a kind:13534 membership roster snapshot.
    AddMember {
        /// Nostr public key — bech32 npub or 64-char hex.
        #[arg(long)]
        pubkey: String,

        /// Role: "admin" or "member" (default: member). Cannot be "owner" —
        /// use RELAY_OWNER_PUBKEY config to set the relay owner.
        #[arg(long, default_value = "member")]
        role: String,
    },
    /// Remove a pubkey from the relay membership list.
    ///
    /// Accepts a bech32 npub or 64-char hex pubkey. After removing the DB row,
    /// stores a kind:13534 membership roster snapshot. Cannot remove the
    /// relay owner — change RELAY_OWNER_PUBKEY config instead.
    RemoveMember {
        /// Nostr public key — bech32 npub or 64-char hex.
        #[arg(long)]
        pubkey: String,

        /// Only remove if the member's current role matches this value.
        /// Omit to remove regardless of role.
        #[arg(long)]
        role: Option<String>,
    },
    /// List all relay members.
    ListMembers,
    /// Generate a new Nostr keypair (for bootstrapping).
    GenerateKey,
    /// Run pending database migrations.
    Migrate,
    /// Inspect deployment-wide Nimino product feedback.
    ProductFeedback {
        #[command(subcommand)]
        command: ProductFeedbackCommand,
    },
    /// Durable CLI-only whole-community deletion control plane.
    Deletions {
        #[command(subcommand)]
        command: deletions::DeletionsCommand,
    },
}

#[derive(Subcommand)]
enum ProductFeedbackCommand {
    /// List feedback across every community as JSON.
    List {
        /// Maximum records to return.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
}

#[tokio::main]
async fn main() {
    // Select the same rustls provider as nimino-relay before any TLS client starts.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();

    let code = match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            5
        }
    };
    std::process::exit(code);
}

async fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::GenerateKey => {
            let keys = Keys::generate();
            println!("Public key:  {}", keys.public_key().to_hex());
            println!("Secret key:  {}", keys.secret_key().display_secret());
            println!("\nSet NIMINO_PRIVATE_KEY to the secret key to use this identity.");
            Ok(0)
        }
        Command::Migrate => {
            let db = connect_db().await?;
            db.migrate().await?;
            println!("Database migrations complete.");
            Ok(0)
        }
        Command::AddMember { pubkey, role } => cmd_add_member(pubkey, role).await,
        Command::RemoveMember { pubkey, role } => cmd_remove_member(pubkey, role).await,
        Command::ListMembers => cmd_list_members().await,
        Command::ProductFeedback {
            command: ProductFeedbackCommand::List { limit },
        } => cmd_list_product_feedback(limit).await,
        Command::Deletions { command } => deletions::run(command).await,
    }
}

async fn cmd_add_member(pubkey_arg: String, role: String) -> Result<i32> {
    if let Err(msg) = validate_role(&role) {
        eprintln!("error: {msg}");
        return Ok(1);
    }

    let pubkey_hex = match parse_pubkey_hex(&pubkey_arg) {
        Ok(h) => h,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    };

    let (db, relay_keypair) = connect_member_services().await?;

    let tenant = resolve_admin_tenant(&db).await?;
    match db
        .add_relay_member(tenant.community(), &pubkey_hex, &role, None)
        .await
    {
        Ok(true) => println!("added {pubkey_hex} as {role}"),
        Ok(false) => println!("already a member: {pubkey_hex} (no change)"),
        Err(e) => {
            eprintln!("error: DB write failed: {e}");
            return Ok(5);
        }
    }

    if let Err(e) = publish_membership_list_with_bump(&db, &relay_keypair, &tenant).await {
        eprintln!("warning: member added to DB but roster snapshot failed: {e}");
    }

    Ok(0)
}

async fn cmd_remove_member(pubkey_arg: String, role_filter: Option<String>) -> Result<i32> {
    if let Some(ref role) = role_filter {
        if let Err(msg) = validate_role(role) {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    }

    let pubkey_hex = match parse_pubkey_hex(&pubkey_arg) {
        Ok(h) => h,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    };

    let (db, relay_keypair) = connect_member_services().await?;

    let tenant = resolve_admin_tenant(&db).await?;
    use nimino_db::relay_members::RemoveResult;
    let result = if let Some(ref role) = role_filter {
        db.remove_relay_member_if_role(tenant.community(), &pubkey_hex, role)
            .await
    } else {
        db.remove_relay_member(tenant.community(), &pubkey_hex)
            .await
    };

    match result {
        Ok(RemoveResult::Removed) => println!("removed {pubkey_hex}"),
        Ok(RemoveResult::NotFound) => {
            eprintln!("error: member not found: {pubkey_hex}");
            return Ok(2);
        }
        Ok(RemoveResult::IsOwner) => {
            eprintln!(
                "error: cannot remove relay owner: {pubkey_hex}\n\
                 To change the owner, update RELAY_OWNER_PUBKEY and restart."
            );
            return Ok(3);
        }
        Ok(RemoveResult::RoleMismatch) => {
            let role_str = role_filter.as_deref().unwrap_or("(unknown)");
            eprintln!("error: role mismatch — {pubkey_hex} is not currently '{role_str}'");
            return Ok(4);
        }
        Err(e) => {
            eprintln!("error: DB write failed: {e}");
            return Ok(5);
        }
    }

    if let Err(e) = publish_membership_list_with_bump(&db, &relay_keypair, &tenant).await {
        eprintln!("warning: member removed from DB but roster snapshot failed: {e}");
    }

    Ok(0)
}

async fn cmd_list_product_feedback(limit: u16) -> Result<i32> {
    let db = connect_db().await?;
    let feedback = db.list_product_feedback(i64::from(limit)).await?;
    println!("{}", serde_json::to_string_pretty(&feedback)?);
    Ok(0)
}

async fn cmd_list_members() -> Result<i32> {
    let db = connect_db().await?;
    let tenant = resolve_admin_tenant(&db).await?;
    let members = db.list_relay_members(tenant.community()).await?;

    if members.is_empty() {
        println!("(no relay members)");
        return Ok(0);
    }

    println!(
        "{:<66} {:<8} {:<66} created_at",
        "pubkey", "role", "added_by"
    );
    println!("{}", "-".repeat(160));
    for m in &members {
        let added_by = m.added_by.as_deref().unwrap_or("-");
        println!(
            "{:<66} {:<8} {:<66} {}",
            m.pubkey,
            m.role,
            added_by,
            m.created_at.format("%Y-%m-%dT%H:%M:%SZ")
        );
    }

    Ok(0)
}

/// Validate that `role` is `"member"` or `"admin"`. Rejects `"owner"`.
fn validate_role(role: &str) -> std::result::Result<(), String> {
    match role {
        "member" | "admin" => Ok(()),
        "owner" => {
            Err("role 'owner' cannot be set via CLI — use RELAY_OWNER_PUBKEY config".to_string())
        }
        other => Err(format!(
            "invalid role '{other}': must be 'member' or 'admin'"
        )),
    }
}

/// Parse a bech32 npub or 64-char hex pubkey into lowercase hex.
fn parse_pubkey_hex(input: &str) -> std::result::Result<String, String> {
    nostr::PublicKey::parse(input)
        .map(|pk| pk.to_hex())
        .map_err(|e| format!("invalid pubkey '{input}': {e}"))
}

/// Publish kind:13534 with `custom_created_at = max(now, newest_existing + 1s)`.
///
/// Guarantees the new event is not dominated by a same-second prior invocation,
/// so `replace_addressable_event` inserts the new snapshot.
///
/// See module-level doc for the TOCTOU caveat on concurrent CLI processes.
async fn publish_membership_list_with_bump(
    db: &Db,
    relay_keypair: &Keys,
    tenant: &TenantContext,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let relay_pubkey = relay_keypair.public_key();
    let relay_pubkey_bytes = relay_pubkey.to_bytes();

    // Query the newest existing kind:13534 for this relay's pubkey (channel_id=None).
    let newest_ts = db
        .get_latest_global_replaceable(
            tenant.community(),
            KIND_NIP43_MEMBERSHIP_LIST as i32,
            &relay_pubkey_bytes,
        )
        .await?
        .map(|e| e.event.created_at.as_secs());

    // custom_created_at = max(now, existing + 1s) — defeats same-second domination.
    let ts = match newest_ts {
        Some(existing) => (existing + 1).max(now),
        None => now,
    };

    let members = db.list_relay_members(tenant.community()).await?;

    let mut tags: Vec<Tag> = Vec::with_capacity(members.len() + 1);
    // NIP-70 protected-event marker — prevents re-broadcasting by third parties.
    tags.push(Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?);
    for member in &members {
        tags.push(
            Tag::parse(["member", &member.pubkey, &member.role])
                .map_err(|e| anyhow::anyhow!("failed to build member tag: {e}"))?,
        );
    }

    let event = EventBuilder::new(Kind::Custom(KIND_NIP43_MEMBERSHIP_LIST as u16), "")
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(ts))
        .sign_with_keys(relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:13534: {e}"))?;

    let (_, was_inserted) = db
        .replace_addressable_event(tenant.community(), &event, None)
        .await?;

    tracing::info!(
        member_count = members.len(),
        ts,
        was_inserted,
        "NIP-43 membership list stored by nimino-admin"
    );
    Ok(())
}

/// Connect to DB and load the relay keypair.
///
/// `NIMINO_RELAY_PRIVATE_KEY` is required — the CLI signs kind:13534 events.
async fn connect_member_services() -> Result<(Db, Keys)> {
    let db = connect_db().await?;

    let relay_keypair = {
        let hex = std::env::var("NIMINO_RELAY_PRIVATE_KEY").map_err(|_| {
            anyhow::anyhow!(
                "NIMINO_RELAY_PRIVATE_KEY is required for add-member/remove-member.\n\
                 The relay must have a stable signing key to publish kind:13534 events."
            )
        })?;
        Keys::parse(&hex).map_err(|e| anyhow::anyhow!("invalid NIMINO_RELAY_PRIVATE_KEY: {e}"))?
    };

    Ok((db, relay_keypair))
}

async fn connect_db() -> Result<Db> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://nimino:nimino_dev@localhost:5432/nimino".to_string());
    let db = Db::new(&DbConfig {
        database_url: db_url,
        ..DbConfig::default()
    })
    .await?;
    Ok(db)
}

/// Resolve the deployment's tenant from the configured `RELAY_URL` host.
///
/// `nimino-admin` runs inside the relay container (`compose exec relay
/// nimino-admin …`), so it shares the relay's `RELAY_URL` and resolves the same
/// single community against the durable `communities` host map. This is
/// deliberately NOT a default tenant: an unmapped host fails closed with an
/// error, mirroring the relay's own `bind_community` row-zero seam. The CLI is
/// single-community per invocation — there is no cross-community sweep.
async fn resolve_admin_tenant(db: &Db) -> Result<TenantContext> {
    let relay_url =
        std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string());
    // Derive the authority the *same* way startup seeding and live request
    // resolution do (`nimino_core::tenant::relay_url_authority`): host plus an
    // explicit non-default port, IPv6 brackets preserved. A plain
    // `Url::host_str()` drops the port/brackets, so for `ws://localhost:3000`
    // the admin would look up `localhost` while startup seeded `localhost:3000`
    // — and `wss://relay.example:8443` would resolve `relay.example`. Sharing
    // the helper keeps nimino-admin byte-identical to the community startup seeds.
    let host = relay_url_authority(&relay_url);
    let record = db.lookup_community_by_host(&host).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "RELAY_URL host '{host}' is not mapped to a community.\n\
             nimino-admin operates on the configured relay's community; ensure the \
             relay has started and seeded its community (or set RELAY_URL to a \
             mapped host)."
        )
    })?;
    Ok(TenantContext::resolved(record.id, record.host))
}
