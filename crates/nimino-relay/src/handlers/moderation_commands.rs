//! Community moderation command handler (kinds 9040–9044, Phase 1 contract).
//!
//! Mirrors the NIP-43 relay-admin pattern (`relay_admin.rs`, 9030-series):
//! commands are validated + executed directly and are **never** stored as
//! regular events. Rust loads scoped facts; Nimino selects authority, transition,
//! and audit action.
//!
//! | Kind | Operation      | Side effects (all mandatory)                       |
//! |------|----------------|----------------------------------------------------|
//! | 9040 | Ban            | `community_bans` upsert, audit row, live disconnect (L4 fanout), restriction notice DM (L5) |
//! | 9041 | Unban          | ban lift, audit row                                |
//! | 9042 | Timeout        | `muted_until` upsert, audit row, notice DM         |
//! | 9043 | Untimeout      | mute clear, audit row                              |
//! | 9044 | Resolve report | report status update, audit row, reporter notice DM; `delete`/`kick`/`ban` resolutions fan out through the existing 9005/9001 + 9040 paths |
//!
//! Targets (`p` tag pubkey, `report` tag row id) are resolved under the
//! request's `TenantContext` only.
//!
//! ## Routing (pinned — Wren contract review, 2026-07-07)
//! 9040–9044 are **community-global direct commands**, exactly like the
//! relay-admin 9030-series: route via
//! [`nimino_core::kind::is_moderation_command_kind`], list them in
//! `is_global_only_kind` so a stray `h` tag can never channel-scope them
//! (no channel membership/archive gates apply), require a fresh timestamp,
//! never store them, and reject channel-scoped API tokens.
//!
//! ## Tag vocabulary (pinned — CLI and relay must agree)
//! - 9040 ban: `["p", <hex pubkey>]` required; optional
//!   `["expiration", <unix secs>]` (absent ⇒ permanent), `["reason", <text>]`.
//! - 9041 unban: `["p", <hex pubkey>]`.
//! - 9042 timeout: `["p", <hex pubkey>]` + required `["expiration", <unix secs>]`;
//!   optional `["reason", <text>]`.
//! - 9043 untimeout: `["p", <hex pubkey>]`.
//! - 9044 resolve (pinned — thread event `86f46207`, 2026-07-07): required,
//!   exactly one each: `["report", <report event id hex>]` (the 1984 report
//!   being resolved; resolves under `tenant.community()` only),
//!   `["status", resolved|dismissed]`,
//!   `["action", delete|kick|ban|timeout|dismiss|escalate]` (`dismiss` pairs
//!   with status `dismissed`; everything else with `resolved`). Optional
//!   `["reason", <text>]` — audited into `moderation_actions.public_reason`
//!   and relayed in the notice DM (so it must be safe for the reporter's
//!   eyes; `private_reason` is mod-only and not fed by 9044 tags). Unknown
//!   extra tags are ignored, not rejected
//!   (forward-compat). `delete`/`kick`/`ban`/`timeout` actions fan out through
//!   the existing 9005/9001 paths and the 9040/9042 handlers — no second
//!   implementation. The resolution audit row records the *decision*, not the
//!   enforcement, so it is prefixed `resolve:` (`resolve:ban`, `resolve:delete`,
//!   …); the client's paired 9040-9043 writes the unprefixed enforcement row.
//!   The `resolve:*` values are part of the DB CHECK vocabulary in migration 0006.
//!   `dismiss` audits as `dismiss_report` and `escalate` as `escalate` (both
//!   unprefixed — escalate must stay queryable for the platform-safety lane).
//!
//! Lane ownership: L6 (Quinn) — plus `nimino-cli` `moderation` command group.
//! The `ingest.rs` routing entries (scope map + `is_global_only_kind` +
//! direct-processing dispatch) for 9040–9044 belong to L3 (Perci):
//! coordinate, don't edit ingest.rs.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use nimino_boundary::{
    MembershipRole, ModerationAuditAction, ModerationEffect, ModerationPolicyError,
    ModerationPolicyRequest, ModerationPolicyResult, ModerationResolutionAction,
    ModerationResolutionRequest, ModerationResolutionStatus, ModerationRestrictionCommand,
    ModerationRestrictionRequest,
};
use nimino_core::kind::{
    KIND_MODERATION_BAN, KIND_MODERATION_RESOLVE_REPORT, KIND_MODERATION_TIMEOUT,
    KIND_MODERATION_UNBAN, KIND_MODERATION_UNTIMEOUT,
};
use nimino_core::tenant::TenantContext;
use nostr::Event;
use tracing::info;
use uuid::Uuid;

use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
use crate::state::AppState;
use nimino_db::moderation::NewAction;

/// Validate and execute a moderation command (kinds 9040–9044).
///
/// Returns a client-safe error string for `OK false` on rejection.
///
/// Routing note: 9040–9044 are community-global direct commands (L3 lists them
/// in `is_global_only_kind`), so no `h`/channel context is consulted here; the
/// tenant is bound from the request. Nimino owns authorization and transitions.
pub async fn handle_moderation_command(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(), String> {
    let kind = event.kind.as_u16() as u32;
    let actor = event.pubkey.to_bytes().to_vec();

    match kind {
        KIND_MODERATION_BAN => handle_ban(tenant, state, event, &actor).await,
        KIND_MODERATION_UNBAN => handle_unban(tenant, state, event, &actor).await,
        KIND_MODERATION_TIMEOUT => handle_timeout(tenant, state, event, &actor).await,
        KIND_MODERATION_UNTIMEOUT => handle_untimeout(tenant, state, event, &actor).await,
        KIND_MODERATION_RESOLVE_REPORT => handle_resolve(tenant, state, event, &actor).await,
        other => Err(invalid(format!(
            "unexpected moderation command kind: {other}"
        ))),
    }
}

fn membership_role(role: Option<&str>) -> MembershipRole {
    match role {
        Some("owner") => MembershipRole::Owner,
        Some("admin") => MembershipRole::Admin,
        Some("member") => MembershipRole::Member,
        Some("guest") => MembershipRole::Guest,
        Some("bot") => MembershipRole::Bot,
        _ => MembershipRole::None,
    }
}

fn audit_action_name(action: ModerationAuditAction) -> &'static str {
    match action {
        ModerationAuditAction::Ban => "ban",
        ModerationAuditAction::Unban => "unban",
        ModerationAuditAction::Timeout => "timeout",
        ModerationAuditAction::Untimeout => "untimeout",
        ModerationAuditAction::DismissReport => "dismiss_report",
        ModerationAuditAction::Escalate => "escalate",
        ModerationAuditAction::ResolveDelete => "resolve:delete",
        ModerationAuditAction::ResolveKick => "resolve:kick",
        ModerationAuditAction::ResolveBan => "resolve:ban",
        ModerationAuditAction::ResolveTimeout => "resolve:timeout",
        ModerationAuditAction::None => "none",
    }
}

async fn decide_restriction(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
    actor: &[u8],
    target: &[u8],
    command: ModerationRestrictionCommand,
    requested_expires_at: Option<i64>,
) -> Result<(ModerationEffect, ModerationAuditAction), String> {
    let actor_hex = hex::encode(actor);
    let target_hex = hex::encode(target);
    let (actor_member, target_member, actor_restriction, target_restriction) = tokio::join!(
        state.db.get_relay_member(tenant.community(), &actor_hex),
        state.db.get_relay_member(tenant.community(), &target_hex),
        state
            .db
            .moderation_restriction_facts(tenant.community(), actor),
        state
            .db
            .moderation_restriction_facts(tenant.community(), target),
    );
    let actor_member =
        actor_member.map_err(|db_error| error(format!("database error: {db_error}")))?;
    let target_member =
        target_member.map_err(|db_error| error(format!("database error: {db_error}")))?;
    let actor_restriction =
        actor_restriction.map_err(|db_error| error(format!("database error: {db_error}")))?;
    let target_restriction =
        target_restriction.map_err(|db_error| error(format!("database error: {db_error}")))?;
    let result = super::ingest::call_moderation_policy(
        state,
        ModerationPolicyRequest::Restriction {
            request: ModerationRestrictionRequest {
                command,
                request_community: *tenant.community().as_uuid(),
                actor_role_community: actor_member.as_ref().map(|_| *tenant.community().as_uuid()),
                target_role_community: target_member
                    .as_ref()
                    .map(|_| *tenant.community().as_uuid()),
                actor_restriction_community: actor_restriction
                    .exists
                    .then_some(*tenant.community().as_uuid()),
                target_restriction_community: target_restriction
                    .exists
                    .then_some(*tenant.community().as_uuid()),
                actor_role: membership_role(
                    actor_member.as_ref().map(|member| member.role.as_str()),
                ),
                target_role: membership_role(
                    target_member.as_ref().map(|member| member.role.as_str()),
                ),
                actor_restriction_exists: actor_restriction.exists,
                actor_ban_set: actor_restriction.ban_set,
                actor_ban_expires_at: actor_restriction
                    .ban_expires_at
                    .map(|value| value.timestamp()),
                target_restriction_exists: target_restriction.exists,
                target_ban_set: target_restriction.ban_set,
                target_ban_expires_at: target_restriction
                    .ban_expires_at
                    .map(|value| value.timestamp()),
                target_muted_until: target_restriction
                    .muted_until
                    .map(|value| value.timestamp()),
                actor_is_target: actor == target,
                created_at_seconds: i64::try_from(event.created_at.as_secs())
                    .map_err(|_| invalid("command timestamp is invalid"))?,
                now_seconds: Utc::now().timestamp(),
                requested_expires_at,
            },
        },
    )
    .await
    .map_err(super::ingest::ingest_error_message)?;
    let ModerationPolicyResult::Restriction {
        effect,
        audit_action,
        error: policy_error,
        ..
    } = result
    else {
        return Err(error("Nim moderation policy returned an unexpected result"));
    };
    if policy_error == ModerationPolicyError::None && effect != ModerationEffect::Reject {
        return Ok((effect, audit_action));
    }
    let message = format!("moderation policy rejected the command ({policy_error:?})");
    match policy_error {
        ModerationPolicyError::ActorBanned => Err(format!("blocked: {message}")),
        ModerationPolicyError::NotAuthorized | ModerationPolicyError::ProtectedTarget => {
            Err(format!("restricted: {message}"))
        }
        _ => Err(invalid(message)),
    }
}

// ── 9040: ban ───────────────────────────────────────────────────────────────

async fn handle_ban(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target = extract_p_tag_bytes(event).ok_or_else(|| invalid("missing or invalid p tag"))?;
    let expires_at = extract_expiration(event)?; // None ⇒ permanent
    let reason = extract_tag_value(event, "reason");

    let (effect, audit_action) = decide_restriction(
        tenant,
        state,
        event,
        actor,
        &target,
        ModerationRestrictionCommand::Ban,
        expires_at.map(|value| value.timestamp()),
    )
    .await?;
    if effect != ModerationEffect::ApplyBan {
        return Err(error(
            "Nim moderation policy selected an invalid ban effect",
        ));
    }

    state
        .db
        .ban_community_member(
            tenant.community(),
            &target,
            actor,
            reason.as_deref(),
            expires_at,
        )
        .await
        .map_err(|e| error(format!("database error: {e}")))?;

    let action_id = insert_audit(
        state,
        tenant,
        actor,
        audit_action_name(audit_action),
        Some(&target),
        None,
        reason.as_deref(),
    )
    .await?;

    // Close this process's open sessions immediately. The durable ban row
    // rejects subsequent authentication and writes.
    state.disconnect_pubkey(
        tenant,
        &target,
        &event.id.to_hex(),
        "blocked: you are banned from this community",
    );

    // Notice DM: tell the banned user the terms of the restriction.
    let public_reason = reason.clone().unwrap_or_default();
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        &target,
        ModerationNotice::Restriction {
            action_id,
            kind: "ban".to_string(),
            public_reason,
        },
    )
    .await
    {
        // Notice delivery is best-effort; the ban itself has already landed and
        // been audited. Log and continue rather than fail the command.
        info!(error = %e, "ban notice DM delivery failed (ban still enforced)");
    }

    info!(target = %hex::encode(&target), "community ban applied");
    Ok(())
}

// ── 9041: unban ──────────────────────────────────────────────────────────────

async fn handle_unban(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target = extract_p_tag_bytes(event).ok_or_else(|| invalid("missing or invalid p tag"))?;

    let (effect, audit_action) = decide_restriction(
        tenant,
        state,
        event,
        actor,
        &target,
        ModerationRestrictionCommand::Unban,
        None,
    )
    .await?;
    if effect != ModerationEffect::LiftBan {
        return Err(error(
            "Nim moderation policy selected an invalid unban effect",
        ));
    }

    let lifted = state
        .db
        .unban_community_member(tenant.community(), &target, actor)
        .await
        .map_err(|e| error(format!("database error: {e}")))?;
    if !lifted {
        return Err(invalid("member is not banned"));
    }

    insert_audit(
        state,
        tenant,
        actor,
        audit_action_name(audit_action),
        Some(&target),
        None,
        None,
    )
    .await?;

    info!(target = %hex::encode(&target), "community ban lifted");
    Ok(())
}

// ── 9042: timeout ────────────────────────────────────────────────────────────

async fn handle_timeout(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target = extract_p_tag_bytes(event).ok_or_else(|| invalid("missing or invalid p tag"))?;
    let muted_until =
        extract_expiration(event)?.ok_or_else(|| invalid("timeout requires an expiration tag"))?;
    let reason = extract_tag_value(event, "reason");

    let (effect, audit_action) = decide_restriction(
        tenant,
        state,
        event,
        actor,
        &target,
        ModerationRestrictionCommand::Timeout,
        Some(muted_until.timestamp()),
    )
    .await?;
    if effect != ModerationEffect::ApplyTimeout {
        return Err(error(
            "Nim moderation policy selected an invalid timeout effect",
        ));
    }

    state
        .db
        .timeout_community_member(
            tenant.community(),
            &target,
            actor,
            muted_until,
            reason.as_deref(),
        )
        .await
        .map_err(|e| error(format!("database error: {e}")))?;

    let action_id = insert_audit(
        state,
        tenant,
        actor,
        audit_action_name(audit_action),
        Some(&target),
        None,
        reason.as_deref(),
    )
    .await?;

    let public_reason = reason.clone().unwrap_or_default();
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        &target,
        ModerationNotice::Restriction {
            action_id,
            kind: "timeout".to_string(),
            public_reason,
        },
    )
    .await
    {
        info!(error = %e, "timeout notice DM delivery failed (timeout still enforced)");
    }

    info!(target = %hex::encode(&target), "community timeout applied");
    Ok(())
}

// ── 9043: untimeout ──────────────────────────────────────────────────────────

async fn handle_untimeout(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target = extract_p_tag_bytes(event).ok_or_else(|| invalid("missing or invalid p tag"))?;

    let (effect, audit_action) = decide_restriction(
        tenant,
        state,
        event,
        actor,
        &target,
        ModerationRestrictionCommand::Untimeout,
        None,
    )
    .await?;
    if effect != ModerationEffect::ClearTimeout {
        return Err(error(
            "Nim moderation policy selected an invalid untimeout effect",
        ));
    }

    let cleared = state
        .db
        .untimeout_community_member(tenant.community(), &target, actor)
        .await
        .map_err(|e| error(format!("database error: {e}")))?;
    if !cleared {
        return Err(invalid("member is not timed out"));
    }

    insert_audit(
        state,
        tenant,
        actor,
        audit_action_name(audit_action),
        Some(&target),
        None,
        None,
    )
    .await?;

    info!(target = %hex::encode(&target), "community timeout cleared");
    Ok(())
}

// ── 9044: resolve report ─────────────────────────────────────────────────────

async fn handle_resolve(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let report_event_id = extract_report_tag(event)
        .ok_or_else(|| invalid("missing or invalid report tag (expect 64-hex event id)"))?;
    let status = extract_tag_value(event, "status").ok_or_else(|| invalid("missing status tag"))?;
    let action = extract_tag_value(event, "action").ok_or_else(|| invalid("missing action tag"))?;
    let reason = extract_tag_value(event, "reason");

    let status_policy = match status.as_str() {
        "resolved" => ModerationResolutionStatus::Resolved,
        "dismissed" => ModerationResolutionStatus::Dismissed,
        _ => return Err(invalid(format!("invalid resolution status: {status}"))),
    };
    let action_policy = match action.as_str() {
        "delete" => ModerationResolutionAction::Delete,
        "kick" => ModerationResolutionAction::Kick,
        "ban" => ModerationResolutionAction::Ban,
        "timeout" => ModerationResolutionAction::Timeout,
        "dismiss" => ModerationResolutionAction::Dismiss,
        "escalate" => ModerationResolutionAction::Escalate,
        _ => return Err(invalid(format!("invalid resolution action: {action}"))),
    };

    // Resolve the report row under this tenant only. The `report` tag carries
    // the signed 1984 event id (pinned contract); look the row up by it.
    let report = state
        .db
        .get_moderation_report_by_event(tenant.community(), &report_event_id)
        .await
        .map_err(|db_error| error(format!("database error: {db_error}")))?;
    let actor_hex = hex::encode(actor);
    let (actor_member, actor_restriction) = tokio::join!(
        state.db.get_relay_member(tenant.community(), &actor_hex),
        state
            .db
            .moderation_restriction_facts(tenant.community(), actor),
    );
    let actor_member =
        actor_member.map_err(|db_error| error(format!("database error: {db_error}")))?;
    let actor_restriction =
        actor_restriction.map_err(|db_error| error(format!("database error: {db_error}")))?;
    let decision = super::ingest::call_moderation_policy(
        state,
        ModerationPolicyRequest::Resolution {
            request: ModerationResolutionRequest {
                request_community: *tenant.community().as_uuid(),
                actor_role_community: actor_member.as_ref().map(|_| *tenant.community().as_uuid()),
                actor_restriction_community: actor_restriction
                    .exists
                    .then_some(*tenant.community().as_uuid()),
                report_community: report.as_ref().map(|_| *tenant.community().as_uuid()),
                actor_role: membership_role(
                    actor_member.as_ref().map(|member| member.role.as_str()),
                ),
                actor_restriction_exists: actor_restriction.exists,
                actor_ban_set: actor_restriction.ban_set,
                actor_ban_expires_at: actor_restriction
                    .ban_expires_at
                    .map(|value| value.timestamp()),
                report_exists: report.is_some(),
                report_open: report
                    .as_ref()
                    .is_some_and(|report| report.status == "open"),
                created_at_seconds: i64::try_from(event.created_at.as_secs())
                    .map_err(|_| invalid("command timestamp is invalid"))?,
                now_seconds: Utc::now().timestamp(),
                status: status_policy,
                action: action_policy,
            },
        },
    )
    .await
    .map_err(super::ingest::ingest_error_message)?;
    let ModerationPolicyResult::Resolution {
        effect,
        audit_action,
        error: policy_error,
        ..
    } = decision
    else {
        return Err(error("Nim moderation policy returned an unexpected result"));
    };
    if policy_error != ModerationPolicyError::None || effect != ModerationEffect::ResolveReport {
        return Err(invalid(format!(
            "moderation policy rejected the resolution ({policy_error:?})"
        )));
    }
    let report = report.ok_or_else(|| error("Nim accepted a missing moderation report"))?;

    // Carry the report's own target into the audit row so `delete`/`kick`/`ban`
    // resolutions record what they acted on.
    let (target_pubkey, target_event_id) = match &report.target {
        nimino_db::moderation::ReportTarget::Pubkey(p) => (Some(p.as_slice()), None),
        nimino_db::moderation::ReportTarget::Event(e) => (None, Some(e.as_slice())),
        nimino_db::moderation::ReportTarget::Blob(_) => (None, None),
    };

    // Distinguish a resolution *decision* from the actual *enforcement* row.
    // A one-click resolve with action=ban records the moderator's decision; the
    // client then composes the real 9040, which writes its own "ban" enforcement
    // row. `resolve:*` decision rows are part of the moderation_actions DB
    // vocabulary so audit consumers can tell the two apart and don't double-count.
    // `dismiss_report` and `escalate` stay unprefixed — escalate especially must
    // remain queryable for the platform-safety lane.
    let action_id = insert_audit(
        state,
        tenant,
        actor,
        audit_action_name(audit_action),
        target_pubkey,
        target_event_id,
        reason.as_deref(),
    )
    .await?;

    let resolved = state
        .db
        .resolve_moderation_report(
            tenant.community(),
            report.id,
            &status,
            actor,
            Some(action_id),
        )
        .await
        .map_err(|e| error(format!("database error: {e}")))?;
    if !resolved {
        return Err(invalid(
            "report is not open (already resolved or dismissed)",
        ));
    }

    // Close the loop: DM the reporter that their report was reviewed.
    let summary = reason.clone().unwrap_or_else(|| match status.as_str() {
        "dismissed" => "Your report was reviewed and dismissed.".to_string(),
        _ => "Your report was reviewed and acted on.".to_string(),
    });
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        &report.reporter_pubkey,
        ModerationNotice::ReportResolved {
            report_id: report.id,
            status: status.clone(),
            summary,
        },
    )
    .await
    {
        info!(error = %e, "report-resolution notice DM delivery failed (report still resolved)");
    }

    info!(report_id = %report.id, status = %status, action = %action, "report resolved");
    Ok(())
}

// ── shared helpers ────────────────────────────────────────────────────────────

/// Insert a moderation audit row for an accepted command. `matched_principal`
/// is left `None` here: that NIP-OA field records which principal an
/// *enforcement* check matched at the auth seam (L4), not who issued a command.
async fn insert_audit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &[u8],
    action: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    public_reason: Option<&str>,
) -> Result<Uuid, String> {
    state
        .db
        .insert_moderation_action(
            tenant.community(),
            NewAction {
                actor_pubkey: actor,
                action,
                target_pubkey,
                target_event_id,
                channel_id: None,
                reason_code: None,
                public_reason,
                private_reason: None,
                matched_principal: None,
            },
        )
        .await
        .map_err(|e| error(format!("failed to write audit row: {e}")))
}

fn invalid(message: impl Into<String>) -> String {
    format!("invalid: {}", message.into())
}

fn error(message: impl Into<String>) -> String {
    format!("error: {}", message.into())
}

/// Extract the first valid `p` tag as raw pubkey bytes (32 bytes).
fn extract_p_tag_bytes(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some("p") {
            if let Some(val) = parts.get(1).map(|s| s.as_str()) {
                if val.len() == 64 && val.chars().all(|c| c.is_ascii_hexdigit()) {
                    return hex::decode(val).ok();
                }
            }
        }
    }
    None
}

/// Extract the `report` tag as a 32-byte event id (the signed 1984 report).
fn extract_report_tag(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some("report") {
            if let Some(val) = parts.get(1).map(|s| s.as_str()) {
                if val.len() == 64 && val.chars().all(|c| c.is_ascii_hexdigit()) {
                    return hex::decode(val).ok();
                }
            }
        }
    }
    None
}

/// Parse an optional `expiration` tag (unix seconds) into a UTC timestamp.
/// Returns `Ok(None)` when absent, `Err` on a malformed value.
fn extract_expiration(event: &Event) -> Result<Option<DateTime<Utc>>, String> {
    match extract_tag_value(event, "expiration") {
        None => Ok(None),
        Some(raw) => {
            let secs: i64 = raw
                .parse()
                .map_err(|_| invalid(format!("invalid expiration tag: {raw}")))?;
            match Utc.timestamp_opt(secs, 0).single() {
                Some(ts) => Ok(Some(ts)),
                None => Err(invalid(format!("expiration out of range: {secs}"))),
            }
        }
    }
}

/// Extract the value of the first tag with the given name.
fn extract_tag_value(event: &Event, name: &str) -> Option<String> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some(name) {
            return parts.get(1).map(|s| s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Build a signed event with the given kind, timestamp, and tags.
    fn make_event(kind: u16, created_at_secs: u64, tags: Vec<Vec<String>>) -> Event {
        let keys = Keys::generate();
        let nostr_tags: Vec<Tag> = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::from(kind), "")
            .tags(nostr_tags)
            .custom_created_at(nostr::Timestamp::from_secs(created_at_secs))
            .sign_with_keys(&keys)
            .expect("signing failed")
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn resolve_audit_actions_are_allowed_by_db_check_vocabulary() {
        for action in [
            ModerationAuditAction::DismissReport,
            ModerationAuditAction::Escalate,
            ModerationAuditAction::ResolveDelete,
            ModerationAuditAction::ResolveKick,
            ModerationAuditAction::ResolveBan,
            ModerationAuditAction::ResolveTimeout,
        ] {
            let audit_action = audit_action_name(action);
            assert!(
                nimino_db::moderation::MODERATION_ACTION_CHECK_VOCAB.contains(&audit_action),
                "{audit_action} must be accepted by migrations/0006_moderation.sql"
            );
        }
    }

    #[test]
    fn command_error_prefix_helpers_preserve_machine_readable_token() {
        assert_eq!(invalid("missing status tag"), "invalid: missing status tag");
        assert_eq!(
            error("database error: connection lost"),
            "error: database error: connection lost"
        );
    }

    #[test]
    fn extract_p_tag_bytes_valid() {
        let hex = "a".repeat(64);
        let e = make_event(9040, now_secs(), vec![vec!["p".into(), hex.clone()]]);
        assert_eq!(extract_p_tag_bytes(&e), hex::decode(&hex).ok());
    }

    #[test]
    fn extract_p_tag_bytes_rejects_short_and_nonhex() {
        assert_eq!(
            extract_p_tag_bytes(&make_event(
                9040,
                now_secs(),
                vec![vec!["p".into(), "abcd".into()]]
            )),
            None
        );
        let bad = "g".repeat(64);
        assert_eq!(
            extract_p_tag_bytes(&make_event(9040, now_secs(), vec![vec!["p".into(), bad]])),
            None
        );
    }

    #[test]
    fn extract_report_tag_requires_64_hex() {
        let id = "b".repeat(64);
        let e = make_event(9044, now_secs(), vec![vec!["report".into(), id.clone()]]);
        assert_eq!(extract_report_tag(&e), hex::decode(&id).ok());
        // A UUID-shaped value (Wren's L5 lesson: never a UUID where an event id belongs).
        let uuid = make_event(
            9044,
            now_secs(),
            vec![vec![
                "report".into(),
                "550e8400-e29b-41d4-a716-446655440000".into(),
            ]],
        );
        assert_eq!(extract_report_tag(&uuid), None);
    }

    #[test]
    fn expiration_absent_is_none() {
        let e = make_event(9040, now_secs(), vec![]);
        assert_eq!(extract_expiration(&e).unwrap(), None);
    }

    #[test]
    fn expiration_valid_parses() {
        let e = make_event(
            9040,
            now_secs(),
            vec![vec!["expiration".into(), "1893456000".into()]],
        );
        assert_eq!(
            extract_expiration(&e).unwrap(),
            Utc.timestamp_opt(1_893_456_000, 0).single()
        );
    }

    #[test]
    fn expiration_malformed_errs() {
        let e = make_event(
            9040,
            now_secs(),
            vec![vec!["expiration".into(), "not-a-number".into()]],
        );
        assert!(extract_expiration(&e).is_err());
    }

    #[test]
    fn expiration_out_of_range_errs() {
        let e = make_event(
            9040,
            now_secs(),
            vec![vec!["expiration".into(), "99999999999999".into()]],
        );
        assert!(extract_expiration(&e).is_err());
    }
}
