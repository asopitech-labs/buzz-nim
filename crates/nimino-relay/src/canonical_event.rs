//! Canonical signed-event persistence before replaceable PostgreSQL projections.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use nimino_core::tenant::CommunityId;
use nimino_store::{
    CanonicalCommit, CommitResult, NodeStorePort, RecordClass, RecordWrite, StoreError,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cluster_runtime::RelayDomainAdapters;
use crate::state::AppState;

const POSTGRES_PROJECTION: &str = "postgres-events-v1";

/// Versioned canonical record type synchronized between Nimino nodes.
pub const RECORD_TYPE: &str = "event";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalEventValue {
    event: nostr::Event,
    channel_id: Option<Uuid>,
    #[serde(default)]
    origin_node_id: String,
    parent_id: String,
    root_id: String,
}

/// Commit one accepted signed event to the per-node canonical store.
///
/// The event id is both the stable record key and idempotency key. Concurrent
/// writers retry only checkpoint conflicts; content reuse is fail-closed.
pub async fn commit(
    domain: &RelayDomainAdapters,
    community_id: CommunityId,
    event: &nostr::Event,
    channel_id: Option<Uuid>,
    parent_id: Option<String>,
    root_id: Option<String>,
) -> Result<CommitResult, StoreError> {
    let store = domain
        .store()
        .ok_or(StoreError::InvalidInput("canonical store is unavailable"))?
        .clone();
    let event_id = event.id.to_hex();
    let value = serde_json::to_value(CanonicalEventValue {
        event: event.clone(),
        channel_id,
        origin_node_id: domain
            .node_id()
            .ok_or(StoreError::InvalidInput("cluster node id is unavailable"))?
            .to_owned(),
        parent_id: parent_id.unwrap_or_default(),
        root_id: root_id.unwrap_or_default(),
    })?;
    let community_id = community_id.to_string();
    tokio::task::spawn_blocking(move || commit_blocking(store, community_id, event_id, value))
        .await
        .map_err(|_| StoreError::Engine("canonical event task failed".to_owned()))?
}

fn commit_blocking(
    store: Arc<dyn NodeStorePort>,
    community_id: String,
    event_id: String,
    value: serde_json::Value,
) -> Result<CommitResult, StoreError> {
    if let Some(existing) = store.get(
        RecordClass::Canonical,
        &community_id,
        RECORD_TYPE,
        &event_id,
    )? {
        let existing: CanonicalEventValue = serde_json::from_value(existing.value)?;
        let incoming: CanonicalEventValue = serde_json::from_value(value.clone())?;
        if existing.event == incoming.event
            && existing.channel_id == incoming.channel_id
            && existing.parent_id == incoming.parent_id
            && existing.root_id == incoming.root_id
        {
            return Ok(CommitResult {
                checkpoint: store.canonical_checkpoint(&community_id)?,
                applied: false,
            });
        }
        return Err(StoreError::IntentConflict);
    }
    for _ in 0..128 {
        let expected_checkpoint = store.canonical_checkpoint(&community_id)?;
        match store.commit_canonical(CanonicalCommit {
            intent_id: format!("event:{event_id}"),
            community_id: community_id.clone(),
            expected_checkpoint,
            writes: vec![RecordWrite {
                record_type: RECORD_TYPE.to_owned(),
                key: event_id.clone(),
                deleted: false,
                value: value.clone(),
            }],
        }) {
            Err(StoreError::CheckpointConflict { .. }) => continue,
            result => return result,
        }
    }
    Err(StoreError::InvalidInput(
        "canonical checkpoint stayed contended",
    ))
}

/// Lifecycle owner for canonical-event projection into the node-local query DB.
pub struct CanonicalProjectionRuntime {
    cancel: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
    ready: Arc<AtomicBool>,
}

impl CanonicalProjectionRuntime {
    /// Rebuild missing query rows before serving, then follow new sync commits.
    pub async fn start(state: Arc<AppState>, ready: Arc<AtomicBool>) -> anyhow::Result<Self> {
        project_all(&state, true).await?;
        ready.store(true, Ordering::Release);
        let cancel = tokio_util::sync::CancellationToken::new();
        let task_cancel = cancel.clone();
        let task_ready = Arc::clone(&ready);
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = task_cancel.cancelled() => break,
                    _ = interval.tick() => match project_all(&state, false).await {
                        Ok(()) => task_ready.store(true, Ordering::Release),
                        Err(error) => {
                            task_ready.store(false, Ordering::Release);
                            tracing::error!(%error, "canonical event projection failed");
                        }
                    }
                }
            }
        });
        Ok(Self {
            cancel,
            task,
            ready,
        })
    }

    /// Stop the projection follower and fail readiness immediately.
    pub async fn stop(self) -> anyhow::Result<()> {
        self.ready.store(false, Ordering::Release);
        self.cancel.cancel();
        self.task
            .await
            .map_err(|error| anyhow::anyhow!("join canonical projection task: {error}"))
    }
}

async fn project_all(state: &Arc<AppState>, recover_local: bool) -> anyhow::Result<()> {
    let communities = state.db.active_community_ids().await?;
    for community in communities {
        while project_batch(state, community, recover_local).await? {}
    }
    Ok(())
}

async fn project_batch(
    state: &Arc<AppState>,
    community_id: CommunityId,
    recover_local: bool,
) -> anyhow::Result<bool> {
    let store = state
        .domain
        .store()
        .ok_or_else(|| anyhow::anyhow!("canonical store is unavailable"))?
        .clone();
    let community = community_id.to_string();
    let cursor_store = store.clone();
    let cursor_community = community.clone();
    let cursor = tokio::task::spawn_blocking(move || {
        cursor_store.projection_checkpoint(&cursor_community, POSTGRES_PROJECTION)
    })
    .await??;
    let changes_store = store.clone();
    let changes_community = community.clone();
    let changes = tokio::task::spawn_blocking(move || {
        changes_store.changes(&changes_community, cursor, nimino_store::MAX_PAGE_SIZE)
    })
    .await??;
    if changes.is_empty() {
        return Ok(false);
    }

    let local_node = state
        .domain
        .node_id()
        .ok_or_else(|| anyhow::anyhow!("cluster node id is unavailable"))?;
    let mut applied = cursor;
    for record in changes {
        if record.record_type == RECORD_TYPE {
            let value: CanonicalEventValue = serde_json::from_value(record.value)?;
            if !recover_local && value.origin_node_id == local_node {
                let exists = state
                    .db
                    .get_event_by_id_including_deleted(
                        community_id,
                        value.event.id.as_bytes().as_slice(),
                    )
                    .await?
                    .is_some();
                if !exists {
                    return Ok(false);
                }
            } else {
                project_event(state, community_id, value).await?;
            }
        }
        let advance_store = store.clone();
        let advance_community = community.clone();
        let next = record.sequence;
        tokio::task::spawn_blocking(move || {
            advance_store.advance_projection_checkpoint(
                &advance_community,
                POSTGRES_PROJECTION,
                applied,
                next,
            )
        })
        .await??;
        applied = next;
    }
    Ok(true)
}

async fn project_event(
    state: &Arc<AppState>,
    community_id: CommunityId,
    value: CanonicalEventValue,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking({
        let event = value.event.clone();
        move || nimino_core::verification::verify_event(&event)
    })
    .await?
    .map_err(|error| anyhow::anyhow!("canonical event signature is invalid: {error}"))?;

    let host = state
        .db
        .lookup_community_host(community_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("canonical event community has no active host"))?;
    let tenant = nimino_core::tenant::TenantContext::resolved(community_id, host);
    let event = value.event;
    let kind = nimino_core::kind::event_kind_u32(&event);

    if nimino_core::kind::is_command_kind(kind) {
        let auth = crate::handlers::ingest::IngestAuth::Http {
            pubkey: event.pubkey,
            scopes: nimino_auth::Scope::all_known(),
            auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
        };
        crate::handlers::command_executor::handle_command(&tenant, state, event, auth)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "canonical command projection failed: {}",
                    crate::handlers::ingest::ingest_error_message(error)
                )
            })?;
        return Ok(());
    }

    let (stored, inserted) = if kind == nimino_core::kind::KIND_REACTION {
        let target = event
            .tags
            .iter()
            .rev()
            .find_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().map(String::as_str) == Some("e"))
                    .then(|| parts.get(1))
                    .flatten()
            })
            .ok_or_else(|| anyhow::anyhow!("canonical reaction has no target"))?;
        let target_id = hex::decode(target)?;
        state
            .db
            .get_event_by_id(community_id, &target_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("canonical reaction target is not projected"))?;
        let actor = crate::handlers::ingest::effective_message_author(
            &event,
            &state.relay_keypair.public_key(),
        );
        match state
            .db
            .insert_reaction_event_with_thread_metadata(
                community_id,
                &event,
                value.channel_id,
                None,
                &target_id,
                &actor,
                event.content.trim(),
            )
            .await?
        {
            nimino_db::ReactionEventInsertOutcome::Inserted {
                stored_event,
                was_inserted,
            } => (*stored_event, was_inserted),
            nimino_db::ReactionEventInsertOutcome::Duplicate => return Ok(()),
            nimino_db::ReactionEventInsertOutcome::TargetMissing => {
                anyhow::bail!("canonical reaction target disappeared during projection")
            }
        }
    } else if nimino_core::kind::is_parameterized_replaceable(kind) {
        let d_tag = nimino_db::event::extract_d_tag(&event).unwrap_or_default();
        state
            .db
            .replace_parameterized_event(community_id, &event, &d_tag, value.channel_id)
            .await?
    } else if nimino_core::kind::is_replaceable(kind) || (39000..=39002).contains(&kind) {
        state
            .db
            .replace_addressable_event(community_id, &event, value.channel_id)
            .await?
    } else {
        let thread = match value.channel_id {
            Some(channel_id) => crate::handlers::ingest::resolve_nip10_thread_meta(
                community_id,
                &event,
                channel_id,
                state,
            )
            .await
            .map_err(|error| anyhow::anyhow!("canonical thread projection failed: {error}"))?,
            None => None,
        };
        state
            .db
            .insert_event_with_thread_metadata(
                community_id,
                &event,
                value.channel_id,
                thread.as_ref().map(|metadata| metadata.as_params()),
            )
            .await?
    };

    if inserted {
        crate::handlers::side_effects::handle_side_effects(&tenant, kind, &event, state).await?;
        crate::handlers::event::dispatch_projected_event(&tenant, state, &stored, kind).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimino_store::{RecordClass, RedbNodeStore};
    use nostr::{EventBuilder, Keys, Kind};

    #[test]
    fn canonical_event_commit_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store: Arc<dyn NodeStorePort> =
            Arc::new(RedbNodeStore::open(temp.path().join("events.redb")).expect("store"));
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        let community = Uuid::new_v4().to_string();
        let id = event.id.to_hex();
        let value = serde_json::to_value(CanonicalEventValue {
            event,
            channel_id: None,
            origin_node_id: "node-a".to_owned(),
            parent_id: String::new(),
            root_id: String::new(),
        })
        .expect("value");

        assert!(
            commit_blocking(store.clone(), community.clone(), id.clone(), value.clone())
                .expect("first")
                .applied
        );
        assert!(
            !commit_blocking(store.clone(), community.clone(), id.clone(), value)
                .expect("replay")
                .applied
        );
        assert!(store
            .get(RecordClass::Canonical, &community, RECORD_TYPE, &id)
            .expect("read")
            .is_some());
    }
}
