#![deny(unsafe_code)]

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use nimino_boundary::{
    EffectReceipt, EffectReceiptOutcome, EffectReconcileCommand, EffectReconcileRequest,
};
use nimino_data_ops::{
    backup_replica, rebuild_projections, reconcile_effect, repair_replica, restore_replica,
    verify_replica, ObjectRepairRoots, ObjectSpec,
};

#[derive(Parser)]
#[command(
    name = "nimino-data-ops",
    about = "Verify or repair one Nimino replica"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Emit canonical, projection, object, and effect health facts as JSON.
    Verify {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        object_root: Option<PathBuf>,
        #[arg(long = "object", value_parser = parse_object)]
        objects: Vec<ObjectSpec>,
    },
    /// Repair an explicitly quarantined target from a selected healthy source.
    Repair {
        #[arg(long)]
        source_store: PathBuf,
        #[arg(long)]
        target_store: PathBuf,
        #[arg(long)]
        quarantine_store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        source_object_root: Option<PathBuf>,
        #[arg(long)]
        target_object_root: Option<PathBuf>,
        #[arg(long)]
        object_quarantine_root: Option<PathBuf>,
        #[arg(long = "object", value_parser = parse_object)]
        objects: Vec<ObjectSpec>,
    },
    /// Create a verified, no-clobber cutover backup bundle.
    Backup {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        object_root: Option<PathBuf>,
        #[arg(long = "object", value_parser = parse_object)]
        objects: Vec<ObjectSpec>,
        #[arg(long)]
        backup_dir: PathBuf,
    },
    /// Restore a verified bundle into new store and object paths.
    Restore {
        #[arg(long)]
        backup_dir: PathBuf,
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        object_root: Option<PathBuf>,
    },
    /// Manually settle or retry one unknown workflow effect.
    EffectReconcile {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        effect_key: String,
        #[arg(long)]
        worker: PathBuf,
        #[arg(long)]
        operator: String,
        #[arg(long)]
        reason: String,
        #[arg(long, value_enum)]
        action: ReconcileAction,
        #[arg(long)]
        receipt_id: Option<String>,
        #[arg(long)]
        result_digest: Option<String>,
    },
    /// Rebuild all replaceable projections from canonical event state.
    ProjectionRebuild {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        worker: PathBuf,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        epoch: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ReconcileAction {
    MarkSucceeded,
    MarkFailed,
    Retry,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let value = match cli.command {
        Command::Verify {
            store,
            community,
            object_root,
            objects,
        } => serde_json::to_value(verify_replica(
            &store,
            &community,
            object_root.as_deref(),
            &objects,
        )?)?,
        Command::Repair {
            source_store,
            target_store,
            quarantine_store,
            community,
            source_object_root,
            target_object_root,
            object_quarantine_root,
            objects,
        } => serde_json::to_value(repair_replica(
            &source_store,
            &target_store,
            &quarantine_store,
            &community,
            ObjectRepairRoots {
                source: source_object_root.as_deref(),
                target: target_object_root.as_deref(),
                quarantine: object_quarantine_root.as_deref(),
            },
            &objects,
        )?)?,
        Command::Backup {
            store,
            community,
            object_root,
            objects,
            backup_dir,
        } => serde_json::to_value(backup_replica(
            &store,
            &community,
            object_root.as_deref(),
            &objects,
            &backup_dir,
        )?)?,
        Command::Restore {
            backup_dir,
            store,
            community,
            object_root,
        } => serde_json::to_value(restore_replica(
            &backup_dir,
            &store,
            object_root.as_deref(),
            &community,
        )?)?,
        Command::EffectReconcile {
            store,
            community,
            effect_key,
            worker,
            operator,
            reason,
            action,
            receipt_id,
            result_digest,
        } => {
            let (command, outcome) = match action {
                ReconcileAction::MarkSucceeded => (
                    EffectReconcileCommand::MarkSucceeded,
                    Some(EffectReceiptOutcome::Succeeded),
                ),
                ReconcileAction::MarkFailed => (
                    EffectReconcileCommand::MarkFailed,
                    Some(EffectReceiptOutcome::Failed),
                ),
                ReconcileAction::Retry => (EffectReconcileCommand::Retry, None),
            };
            let receipt = match outcome {
                Some(outcome) => Some(EffectReceipt {
                    outcome,
                    receipt_id: receipt_id
                        .ok_or_else(|| anyhow::anyhow!("--receipt-id is required"))?,
                    result_digest: result_digest
                        .ok_or_else(|| anyhow::anyhow!("--result-digest is required"))?,
                }),
                None if receipt_id.is_none() && result_digest.is_none() => None,
                None => anyhow::bail!("retry does not accept receipt fields"),
            };
            serde_json::to_value(
                reconcile_effect(
                    &store,
                    &community,
                    &effect_key,
                    &worker,
                    EffectReconcileRequest {
                        operator_authorized: true,
                        operator_id: operator,
                        reason,
                        command,
                        receipt,
                    },
                )
                .await?,
            )?
        }
        Command::ProjectionRebuild {
            store,
            community,
            worker,
            owner,
            epoch,
        } => serde_json::to_value(
            rebuild_projections(&store, &community, &worker, &owner, &epoch).await?,
        )?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn parse_object(value: &str) -> Result<ObjectSpec, String> {
    let Some((digest, size)) = value.split_once(':') else {
        return Err("object must be DIGEST:SIZE".to_owned());
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("object digest must be lowercase SHA-256".to_owned());
    }
    let size = size
        .parse::<u64>()
        .map_err(|_| "object size must be an integer".to_owned())?;
    if size == 0 {
        return Err("object size must be positive".to_owned());
    }
    Ok(ObjectSpec {
        digest: digest.to_owned(),
        size,
    })
}
