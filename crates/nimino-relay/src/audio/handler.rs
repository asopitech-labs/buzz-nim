//! WebSocket audio handler: NIP-42 auth → room join → frame relay → cleanup.
//!
//! ```text
//! ws_audio_handler
//!   └─ handle_audio_connection
//!        ├─ send challenge, await auth (5s timeout)
//!        ├─ ensure_membership (auto-add for ephemeral channels)
//!        ├─ room.add_peer → broadcast joined
//!        ├─ spawn send_loop + heartbeat_loop
//!        ├─ run recv_loop (blocks until disconnect)
//!        └─ cleanup: remove peer, broadcast left, emit lifecycle events
//! ```

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::http::{HeaderMap, StatusCode};
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Kind, Tag};
use serde::Deserialize;
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

use nimino_auth::generate_challenge;
use nimino_boundary::{MembershipAction, MembershipCommand};
use nimino_core::tenant::TenantContext;
use nimino_db::channel::MemberRole;

use nimino_core::StoredEvent;

use crate::audio::room::PeerCtrl;
use crate::state::{run_registered_community_connection, AppState, CommunityConnectionControl};

/// Maximum binary frame size: 4 KB is generous for a single Opus packet.
const MAX_AUDIO_FRAME_BYTES: usize = 4096;

/// Maximum text frame size: 8 KB bounds auth/control JSON parsing.
const MAX_TEXT_FRAME_BYTES: usize = 8192;

/// Parser-level cap for this route. Text auth/control frames are the largest
/// message type audio accepts; binary Opus frames are bounded more tightly
/// after parsing.
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = MAX_TEXT_FRAME_BYTES;

/// Heartbeat interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Missed pong limit before disconnect.
const MAX_MISSED_PONGS: u8 = 3;

/// Auth timeout.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// WebSocket upgrade handler for `/huddle/:channel_id/audio`.
pub async fn ws_audio_handler(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Row zero: bind this huddle-audio connection to its community from the
    // request host BEFORE the WebSocket upgrade, identical to the main relay
    // door. An unmapped host or lookup failure fails closed with a generic 404
    // — never a default tenant — so an unauthenticated caller cannot probe
    // which communities exist on this deployment.
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = match crate::tenant::bind_community(&state.db, raw_host).await {
        Ok(ctx) => ctx,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
                .into_response();
        }
    };

    let permit = match acquire_audio_connection_permit(&state.conn_semaphore) {
        Some(permit) => permit,
        None => {
            warn!(channel_id = %channel_id, "Connection limit reached, rejecting audio WebSocket");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "relay: connection limit reached",
            )
                .into_response();
        }
    };

    // Keep the parser boundary at the largest message this route accepts. The
    // checks in the receive loop still distinguish text from binary policy, but
    // they run after tungstenite has assembled a message.
    limit_audio_websocket(ws).on_upgrade(move |socket| {
        handle_audio_connection(socket, state, tenant, channel_id, permit)
    })
}

fn acquire_audio_connection_permit(
    conn_semaphore: &Arc<Semaphore>,
) -> Option<OwnedSemaphorePermit> {
    Arc::clone(conn_semaphore).try_acquire_owned().ok()
}

fn limit_audio_websocket<F>(ws: WebSocketUpgrade<F>) -> WebSocketUpgrade<F> {
    ws.max_message_size(MAX_WEBSOCKET_MESSAGE_BYTES)
        .max_frame_size(MAX_WEBSOCKET_MESSAGE_BYTES)
}

/// Huddle audio protocol range accepted by Nimino.
const MIN_PROTOCOL_VERSION: u8 = 2;
const CURRENT_PROTOCOL_VERSION: u8 = 3;

#[derive(Deserialize)]
struct AuthMsg {
    #[serde(rename = "type")]
    msg_type: String,
    event: nostr::Event,
    parent_channel_id: Option<Uuid>,
    /// Huddle audio protocol version requested by the client. This is required;
    /// a room is pinned to its first peer's version.
    protocol_version: u8,
}

async fn handle_audio_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    channel_id: Uuid,
    _permit: OwnedSemaphorePermit,
) {
    let cancel = CancellationToken::new();
    let control = CommunityConnectionControl::new(cancel);
    let community_id = tenant.community();
    let registry = Arc::clone(&state.community_connections);
    let check_state = Arc::clone(&state);
    let run_state = Arc::clone(&state);
    run_registered_community_connection(
        &registry,
        Uuid::new_v4(),
        community_id,
        control,
        move || async move { check_state.db.is_community_active(community_id).await },
        move |control| {
            handle_active_audio_connection(socket, run_state, tenant, channel_id, control)
        },
    )
    .await;
}

async fn handle_active_audio_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    channel_id: Uuid,
    control: CommunityConnectionControl,
) {
    let cancel = control.cancellation_token();
    let disconnect_reason = control.disconnect_reason();
    let (mut ws_send, mut ws_recv) = socket.split();

    let challenge = generate_challenge();
    let challenge_msg =
        serde_json::json!({"type": "challenge", "challenge": challenge}).to_string();
    if ws_send
        .send(WsMessage::Text(challenge_msg.into()))
        .await
        .is_err()
    {
        return;
    }

    let auth_result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return,
        result = tokio::time::timeout(AUTH_TIMEOUT, async {
            while let Some(Ok(msg)) = ws_recv.next().await {
                if let WsMessage::Text(text) = msg {
                    if text.len() > MAX_TEXT_FRAME_BYTES {
                        warn!(channel_id = %channel_id, "auth text frame too large — dropping");
                        continue;
                    }
                    if let Ok(auth) = serde_json::from_str::<AuthMsg>(&text) {
                        if auth.msg_type == "auth" {
                            return Some(auth);
                        }
                    }
                }
            }
            None
        }) => result,
    };

    let auth_msg = match auth_result {
        Ok(Some(a)) => a,
        _ => {
            debug!(channel_id = %channel_id, "audio auth timeout or disconnect");
            return;
        }
    };

    // Extract NIP-OA auth tag before verify_auth_event consumes the event.
    let auth_tag_json = crate::handlers::auth::extract_auth_tag_json(&auth_msg.event);

    let relay_url = crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &tenant);
    let auth_ctx = match state
        .auth
        .verify_auth_event(auth_msg.event, &challenge, &relay_url)
        .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!(channel_id = %channel_id, "audio auth failed: {e}");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"auth failed"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let pubkey = auth_ctx.pubkey;
    let pubkey_hex = pubkey.to_hex();
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let parent_channel_id = auth_msg.parent_channel_id;

    if crate::api::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag_json.as_deref(),
    )
    .await
    .is_err()
    {
        warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "audio: relay membership denied");
        let _ = ws_send
            .send(WsMessage::Text(
                serde_json::json!({"type": "error", "message": "restricted: not a relay member"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    }

    // ── Step 3: membership check / auto-add ───────────────────────────────────
    let parent_id_for_event = match ensure_membership(
        &state,
        &tenant,
        channel_id,
        &pubkey_bytes,
        parent_channel_id,
    )
    .await
    {
        Ok(parent_id) => parent_id,
        Err(e) => {
            warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "audio membership denied: {e}");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"not a member"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    if !state.config.huddle_audio_available {
        debug!(
            channel_id = %channel_id,
            pubkey = %pubkey_hex,
            "node-local huddle audio disabled — rejecting join"
        );
        let _ = ws_send
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "error",
                    "code": "huddle_audio_unavailable",
                    "message": "huddle audio unavailable in this deployment"
                })
                .to_string()
                .into(),
            ))
            .await;
        return;
    }

    let room = state
        .audio_rooms
        .get_or_create(tenant.community(), channel_id);

    // Re-check archived status after obtaining the room. This closes the
    // cross-boundary race: a joiner that passed ensure_membership before
    // the last peer archived the channel could get a fresh room via
    // get_or_create (the old room was already cleaned up). This DB check
    // catches that case. The room-level ended flag (checked inside add_peer)
    // handles the same-room case.
    match state.db.get_channel(tenant.community(), channel_id).await {
        Ok(ch) if ch.archived_at.is_some() => {
            debug!(channel_id = %channel_id, "channel archived before room join");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                ))
                .await;
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            return;
        }
        Err(e) => {
            warn!(channel_id = %channel_id, "pre-join channel check failed (fail-closed): {e}");
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            return;
        }
        Ok(_) => {} // Channel exists and is not archived — proceed.
    }

    // Reject predecessor and future versions before pinning a room.
    let requested_version = auth_msg.protocol_version;
    if !(MIN_PROTOCOL_VERSION..=CURRENT_PROTOCOL_VERSION).contains(&requested_version) {
        warn!(
            channel_id = %channel_id,
            pubkey = %pubkey_hex,
            requested_version,
            current = CURRENT_PROTOCOL_VERSION,
            "audio: client requested unsupported protocol version"
        );
        let _ = ws_send
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "error",
                    "code": "unsupported_version",
                    "message": format!(
                        "huddle audio protocol v{requested_version} not supported; Nimino accepts v{MIN_PROTOCOL_VERSION} through v{CURRENT_PROTOCOL_VERSION}"
                    ),
                    "minimum_version": MIN_PROTOCOL_VERSION,
                    "current_version": CURRENT_PROTOCOL_VERSION,
                })
                .to_string()
                .into(),
            ))
            .await;
        return;
    }

    let admission = room.add_peer(pubkey_hex.clone(), requested_version);
    let (peer_id, peer_index, peer_epoch, audio_rx, peer_ctrl_rx, admission_revision) =
        match admission {
            Ok(v) => v,
            Err(crate::audio::room::AdmissionError::Full) => {
                warn!(channel_id = %channel_id, "audio room participant capacity reached");
                let _ = ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_full","message":"room participant capacity reached"}).to_string().into())).await;
                return;
            }
            Err(crate::audio::room::AdmissionError::Ended) => {
                debug!(channel_id = %channel_id, "room ended before admission");
                let _ = ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_ended","message":"huddle has ended"}).to_string().into())).await;
                return;
            }
            Err(crate::audio::room::AdmissionError::VersionMismatch { pinned, requested }) => {
                info!(channel_id = %channel_id, pubkey = %pubkey_hex, pinned, requested, "audio: protocol version mismatch — upgrade required");
                let _ = ws_send.send(WsMessage::Text(serde_json::json!({
                "type": "error", "code": "upgrade_required",
                "message": format!("this huddle is using audio protocol v{pinned}; your client requested v{requested}"),
                "pinned_version": pinned, "requested_version": requested,
            }).to_string().into())).await;
                return;
            }
        };

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        peer_index,
        "audio peer joined"
    );

    let snapshot = room.roster_snapshot();
    let peers_snapshot: Vec<serde_json::Value> = snapshot
        .peers
        .into_iter()
        .map(|peer| {
            serde_json::json!({"pubkey": peer.pubkey, "peer_index": peer.peer_index, "epoch": peer.epoch})
        })
        .collect();
    let roster_revision = snapshot.revision;
    debug_assert!(roster_revision >= admission_revision);

    let joined_msg = serde_json::json!({
        "type": "joined",
        "revision": roster_revision,
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
        "epoch": peer_epoch,
        "peers": peers_snapshot,
    })
    .to_string();

    room.broadcast_control(joined_msg);

    // ── Step 6: emit kind:48101 (PARTICIPANT_JOINED) ──────────────────────────
    emit_participant_event(
        &state,
        &tenant,
        channel_id,
        parent_id_for_event,
        ParticipantLifecycle {
            kind: Kind::Custom(48101),
            participant_pubkey: &pubkey_hex,
            roster_revision: Some(admission_revision),
            admission_id: Some(peer_id),
        },
    )
    .await;

    let missed_pongs = Arc::new(AtomicU8::new(0));

    // Dual-channel pattern (matches connection.rs): data channel for audio,
    // control channel for Ping/Pong/Close/control JSON with priority drain.
    let (data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);

    let send_cancel = cancel.child_token();
    let send_task = tokio::spawn(send_loop(
        ws_send,
        data_rx,
        ctrl_rx,
        send_cancel,
        disconnect_reason,
    ));

    let hb_cancel = cancel.clone();
    let hb_missed = Arc::clone(&missed_pongs);
    let heartbeat_task = tokio::spawn(heartbeat_loop(ctrl_tx.clone(), hb_missed, hb_cancel));

    let fwd_cancel = cancel.child_token();
    let forward_task = tokio::spawn(audio_forward_loop(
        audio_rx,
        peer_ctrl_rx,
        data_tx,
        ctrl_tx.clone(),
        fwd_cancel,
        cancel.clone(),
    ));

    recv_loop(
        ws_recv,
        Arc::clone(&room),
        peer_id,
        requested_version,
        ctrl_tx,
        Arc::clone(&missed_pongs),
        cancel.clone(),
    )
    .await;

    cancel.cancel();
    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = forward_task.await;
    let removal = room.remove_peer_and_check_ended(peer_id);
    let removal_revision = removal.as_ref().map(|(delta, _)| delta.revision);
    let should_auto_end = removal.as_ref().map(|(_, ended)| *ended).unwrap_or(false);

    if let Some((delta, _)) = removal {
        if let Some(left) = delta.left {
            let left_msg = serde_json::json!({
                "type": "left",
                "revision": delta.revision,
                "pubkey": left.pubkey,
                "peer_index": left.peer_index,
                "epoch": left.epoch,
            })
            .to_string();
            room.broadcast_control(left_msg);
        } else {
            warn!(
                channel_id = %channel_id,
                revision = delta.revision,
                "audio peer removal delta did not include the removed peer"
            );
        }
    }

    emit_participant_event(
        &state,
        &tenant,
        channel_id,
        parent_id_for_event,
        ParticipantLifecycle {
            kind: Kind::Custom(48102),
            participant_pubkey: &pubkey_hex,
            roster_revision: removal_revision,
            admission_id: Some(peer_id),
        },
    )
    .await;

    if should_auto_end {
        info!(channel_id = %channel_id, "audio room empty — auto-ending huddle");

        match state
            .db
            .archive_channel(tenant.community(), channel_id)
            .await
        {
            Err(e) => {
                warn!(channel_id = %channel_id, "auto-archive failed, huddle stays alive: {e}");
                room.clear_ended();
            }
            Ok(()) => {
                state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id);

                emit_participant_event(
                    &state,
                    &tenant,
                    channel_id,
                    parent_id_for_event,
                    ParticipantLifecycle {
                        kind: Kind::Custom(48103),
                        participant_pubkey: &pubkey_hex,
                        roster_revision: None,
                        admission_id: None,
                    },
                )
                .await;
            }
        }
    } else {
        state
            .audio_rooms
            .cleanup_if_empty(tenant.community(), channel_id);
    }

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        "audio peer left"
    );
}

/// Receive loop: reads client frames and routes them through the local room.
#[allow(clippy::too_many_arguments)]
async fn recv_loop(
    mut ws_recv: futures_util::stream::SplitStream<WebSocket>,
    room: Arc<crate::audio::room::Room>,
    peer_id: Uuid,
    protocol_version: u8,
    ctrl_tx: mpsc::Sender<WsMessage>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
) {
    use crate::audio::wire::{FrameHeader, V2_HEADER_LEN};

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            msg = ws_recv.next() => {
                match msg {
                    Some(Ok(WsMessage::Binary(data))) => {
                        if data.len() > MAX_AUDIO_FRAME_BYTES {
                            warn!(peer_id = %peer_id, bytes = data.len(), "audio frame too large — dropping");
                            continue;
                        }

                        // Protocol v2 sanity-parse: validate the header is
                        // present and well-shaped, then forward opaquely.
                        // We never strip, rewrite, or re-encode bytes — the
                        // header is sender-authored telemetry only — but we
                        // do refuse to broadcast frames that are clearly
                        // malformed for the room's pinned protocol so we
                        // don't help v2 peers feed garbage to other v2 peers.
                        if protocol_version >= 2 {
                            // Frame must carry at least the 8-byte header
                            // plus a non-empty Opus payload.
                            if data.len() <= V2_HEADER_LEN {
                                warn!(
                                    peer_id = %peer_id,
                                    bytes = data.len(),
                                    "v2 frame missing header or payload — dropping"
                                );
                                continue;
                            }
                            match FrameHeader::parse(&data) {
                                Some((header, payload)) if !payload.is_empty() => {
                                    // Header is well-formed. `level_dbov` is
                                    // already clamped by `parse` — bad values
                                    // do not drop the frame, they just lose
                                    // the metric (which the relay does not
                                    // trust for anything anyway).
                                    tracing::trace!(
                                        peer_id = %peer_id,
                                        seq = header.seq,
                                        ts_48k = header.ts_48k,
                                        level_dbov = header.level_dbov,
                                        is_dtx = header.is_dtx(),
                                        "v2 audio frame"
                                    );
                                }
                                _ => {
                                    warn!(
                                        peer_id = %peer_id,
                                        bytes = data.len(),
                                        "v2 frame failed header parse — dropping"
                                    );
                                    continue;
                                }
                            }
                        }

                        room.broadcast_frame(peer_id, data);
                    }
                    Some(Ok(WsMessage::Text(text))) => {
                        if text.len() > MAX_TEXT_FRAME_BYTES {
                            warn!(peer_id = %peer_id, bytes = text.len(), "control text frame too large — dropping");
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            if v.get("type").and_then(|t| t.as_str()) == Some("leave") {
                                break;
                            }
                        }
                    }
                    Some(Ok(WsMessage::Pong(_))) => {
                        missed_pongs.store(0, Ordering::Relaxed);
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        // Pong goes through the control channel — priority delivery.
                        let _ = ctrl_tx.try_send(WsMessage::Pong(data));
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Err(e)) => {
                        debug!(peer_id = %peer_id, "ws error: {e}");
                        break;
                    }
                }
            }
        }
    }
}

/// Outbound send loop with control-frame priority (matches connection.rs pattern).
///
/// Control frames (Ping, Pong, Close, control JSON) are drained first on every
/// iteration, so heartbeat pings are never starved by audio backpressure.
async fn send_loop<S>(
    mut ws_send: S,
    mut data_rx: mpsc::Receiver<WsMessage>,
    mut ctrl_rx: mpsc::Receiver<WsMessage>,
    cancel: CancellationToken,
    disconnect_reason: watch::Receiver<Option<crate::state::CommunityDisconnectReason>>,
) where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    loop {
        // Priority: drain all pending control frames before data.
        while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
            if ws_send.send(ctrl_msg).await.is_err() {
                return;
            }
        }

        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let close = disconnect_reason
                    .borrow()
                    .map_or(WsMessage::Close(None), |reason| reason.close_message());
                let _ = ws_send.send(close).await;
                break;
            }
            Some(ctrl_msg) = ctrl_rx.recv() => {
                if ws_send.send(ctrl_msg).await.is_err() { break; }
            }
            Some(msg) = data_rx.recv() => {
                if ws_send.send(msg).await.is_err() { break; }
            }
        }
    }
}

// Bridges the room's mpsc channel to the WS send channel.

/// Bridges room per-peer channels → WS send channels.
/// Audio frames (from room audio_rx) go to data_tx.
/// Control messages (from room ctrl_rx) go to ws ctrl_tx (priority path).
/// Two separate room channels ensure control is never starved by audio backpressure.
async fn audio_forward_loop(
    mut audio_rx: mpsc::Receiver<Bytes>,
    mut peer_ctrl_rx: mpsc::Receiver<PeerCtrl>,
    data_tx: mpsc::Sender<WsMessage>,
    ctrl_tx: mpsc::Sender<WsMessage>,
    cancel: CancellationToken,
    connection_cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            // Control messages get priority over audio in the select.
            msg = peer_ctrl_rx.recv() => {
                match msg {
                    Some(PeerCtrl::Json(json)) => {
                        if ctrl_tx.try_send(WsMessage::Text(json.into())).is_err() {
                            // State-bearing roster control may not be dropped.
                            // Closing the connection forces admission to replay
                            // a fresh authoritative snapshot.
                            connection_cancel.cancel();
                            break;
                        }
                    }
                    Some(PeerCtrl::Close) | None => {
                        connection_cancel.cancel();
                        break;
                    }
                }
            }
            frame = audio_rx.recv() => {
                match frame {
                    Some(bytes) => {
                        let _ = data_tx.try_send(WsMessage::Binary(bytes));
                    }
                    None => break,
                }
            }
        }
    }
}

async fn heartbeat_loop(
    ws_tx: mpsc::Sender<WsMessage>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // fetch_add returns the previous value; +1 gives the current count.
                let missed = missed_pongs.fetch_add(1, Ordering::Relaxed) + 1;
                if missed >= MAX_MISSED_PONGS {
                    warn!("audio: {missed} missed pongs — closing connection");
                    cancel.cancel();
                    break;
                }
                if ws_tx.try_send(WsMessage::Ping(axum::body::Bytes::new())).is_err() {
                    cancel.cancel();
                    break;
                }
            }
            _ = cancel.cancelled() => break,
        }
    }
}

async fn ensure_membership(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    pubkey_bytes: &[u8],
    parent_channel_id: Option<Uuid>,
) -> Result<Uuid, String> {
    // Load channel first — reject archived channels before any membership check.
    // This ensures auto-ended huddles can't be rejoined by existing members.
    let channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if channel.archived_at.is_some() {
        return Err("channel is archived".into());
    }

    // Lifecycle events for an ephemeral huddle belong in its parent channel.
    // Resolve that parent from a creator-signed kind:48100 event instead of
    // trusting the UUID supplied by the client during audio auth.
    let lifecycle_parent_id = if channel.ttl_seconds.is_some() {
        let parent_id = parent_channel_id.ok_or("ephemeral channel requires parent linkage")?;
        let linked = state
            .db
            .huddle_started_link_exists(
                tenant.community(),
                parent_id,
                channel_id,
                &channel.created_by,
            )
            .await
            .map_err(|e| format!("db error: {e}"))?;
        if !linked {
            return Err("ephemeral channel is not linked to claimed parent".into());
        }
        parent_id
    } else {
        channel_id
    };

    // Fast path: already a member.
    let is_member = state
        .is_member_cached(tenant.community(), channel_id, pubkey_bytes)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if is_member {
        return Ok(lifecycle_parent_id);
    }

    if channel.visibility == "open" {
        return Ok(lifecycle_parent_id);
    }

    // Auto-add path: private ephemeral channel + caller is member of parent.
    if channel.ttl_seconds.is_some() {
        let parent_member = state
            .is_member_cached(tenant.community(), lifecycle_parent_id, pubkey_bytes)
            .await
            .map_err(|e| format!("db error: {e}"))?;

        if parent_member {
            let (action, role) = crate::handlers::side_effects::decide_channel_membership(
                tenant,
                state,
                channel_id,
                &channel.created_by,
                pubkey_bytes,
                MembershipCommand::Add,
                Some(MemberRole::Member),
            )
            .await
            .map_err(|error| format!("auto-add policy failed: {error}"))?;
            if action == MembershipAction::Noop {
                return Ok(lifecycle_parent_id);
            }
            state
                .db
                .add_member(
                    tenant.community(),
                    channel_id,
                    pubkey_bytes,
                    role,
                    Some(&channel.created_by),
                )
                .await
                .map_err(|e| format!("auto-add failed: {e}"))?;
            state.invalidate_membership(tenant, channel_id, pubkey_bytes);

            return Ok(lifecycle_parent_id);
        }
    }

    Err("not a member".into())
}

#[derive(Clone, Copy)]
struct ParticipantLifecycle<'a> {
    kind: Kind,
    participant_pubkey: &'a str,
    roster_revision: Option<u64>,
    admission_id: Option<Uuid>,
}

async fn emit_participant_event(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    parent_channel_id: Uuid,
    lifecycle: ParticipantLifecycle<'_>,
) {
    let ParticipantLifecycle {
        kind,
        participant_pubkey,
        roster_revision,
        admission_id,
    } = lifecycle;
    let content = match (roster_revision, admission_id) {
        (Some(revision), Some(admission_id)) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "roster_revision": revision,
            "admission_id": admission_id.to_string(),
        }),
        (Some(revision), None) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "roster_revision": revision,
        }),
        (None, Some(admission_id)) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "admission_id": admission_id.to_string(),
        }),
        (None, None) => serde_json::json!({"ephemeral_channel_id": channel_id.to_string()}),
    }
    .to_string();

    let h_tag = match Tag::parse(["h", &parent_channel_id.to_string()]) {
        Ok(t) => t,
        Err(e) => {
            warn!("audio: failed to parse h tag: {e}");
            return;
        }
    };
    let p_tag = match Tag::parse(["p", participant_pubkey]) {
        Ok(t) => t,
        Err(e) => {
            warn!("audio: failed to parse p tag: {e}");
            return;
        }
    };
    let tags = vec![h_tag, p_tag];

    let event = match EventBuilder::new(kind, content)
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
    {
        Ok(e) => e,
        Err(e) => {
            warn!("audio: failed to sign lifecycle event: {e}");
            return;
        }
    };

    let event_id_hex = event.id.to_hex();

    // 1. Persist to DB so late-joining clients can reconstruct huddle state
    //    from historical queries. Without this, lifecycle events only exist
    //    for the duration of the process-local delivery delivery and are lost forever.
    let stored = match state
        .db
        .insert_event(tenant.community(), &event, Some(parent_channel_id))
        .await
    {
        Ok((stored, true)) => stored,
        Ok((_, false)) => {
            // Duplicate — already persisted (e.g. concurrent emit). Skip fan-out
            // to avoid double-delivery, matching the side_effects.rs pattern.
            debug!(
                event_id = %event_id_hex,
                channel_id = %parent_channel_id,
                "audio lifecycle event already persisted — skipping fan-out"
            );
            return;
        }
        Err(e) => {
            // DB failure during disconnect cleanup. Still broadcast so live
            // subscribers see the leave/end event immediately — suppressing it
            // would leave connected clients stale. Late joiners will have an
            // inconsistent view until the next huddle lifecycle event lands.
            warn!(
                event_id = %event_id_hex,
                channel_id = %parent_channel_id,
                kind = %event.kind.as_u16(),
                "audio: failed to persist lifecycle event: {e}"
            );
            StoredEvent::new(event.clone(), Some(parent_channel_id))
        }
    };

    // 2. Local fan-out to WS subscribers on this node, through the guarded send
    //    path so a stale subscription on a removed/non-member connection cannot
    //    receive this channel's audio lifecycle event (same gate as
    //    dispatch_persistent_event in the ingest handler).
    crate::handlers::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored)
        .await;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::{routing::get, Router};
    use futures_util::SinkExt;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    use super::*;

    #[test]
    fn audio_connection_permits_share_the_global_websocket_budget() {
        let semaphore = Arc::new(Semaphore::new(1));
        let first = acquire_audio_connection_permit(&semaphore).expect("first permit");

        assert!(
            acquire_audio_connection_permit(&semaphore).is_none(),
            "audio connections must stop when the global WebSocket budget is exhausted"
        );

        drop(first);
        assert!(
            acquire_audio_connection_permit(&semaphore).is_some(),
            "dropping an audio connection must return its global permit"
        );
    }

    async fn handler_receives_message_of_size(size: usize) -> bool {
        let (received_tx, received_rx) = oneshot::channel();
        let received_tx = Arc::new(Mutex::new(Some(received_tx)));
        let app = Router::new().route(
            "/",
            get({
                let received_tx = Arc::clone(&received_tx);
                move |ws: WebSocketUpgrade| {
                    let received_tx = Arc::clone(&received_tx);
                    async move {
                        limit_audio_websocket(ws).on_upgrade(move |mut socket| async move {
                            let received = matches!(socket.recv().await, Some(Ok(_)));
                            if let Some(tx) =
                                received_tx.lock().expect("result lock poisoned").take()
                            {
                                let _ = tx.send(received);
                            }
                        })
                    }
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test WebSocket listener");
        let addr = listener.local_addr().expect("test listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test WebSocket server");
        });

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect test WebSocket client");
        client
            .send(Message::Text("x".repeat(size).into()))
            .await
            .expect("send test WebSocket message");

        let received = tokio::time::timeout(Duration::from_secs(2), received_rx)
            .await
            .expect("server should process the test message")
            .expect("server should report whether it received the message");

        server.abort();
        let _ = server.await;

        received
    }

    #[tokio::test]
    async fn saturated_websocket_control_queue_cancels_the_audio_connection() {
        let (_audio_tx, audio_rx) = mpsc::channel(1);
        let (peer_ctrl_tx, peer_ctrl_rx) = mpsc::channel(2);
        let (data_tx, _data_rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        ctrl_tx
            .try_send(WsMessage::Ping(Bytes::new()))
            .expect("fill websocket control queue");
        peer_ctrl_tx
            .try_send(PeerCtrl::Json("{}".into()))
            .expect("queue state-bearing control");
        let task_cancel = CancellationToken::new();
        let connection_cancel = CancellationToken::new();

        audio_forward_loop(
            audio_rx,
            peer_ctrl_rx,
            data_tx,
            ctrl_tx,
            task_cancel,
            connection_cancel.clone(),
        )
        .await;

        assert!(
            connection_cancel.is_cancelled(),
            "saturated websocket control must force a fresh roster admission"
        );
    }

    #[tokio::test]
    async fn closed_peer_control_queue_cancels_the_audio_connection() {
        let (_audio_tx, audio_rx) = mpsc::channel(1);
        let (peer_ctrl_tx, peer_ctrl_rx) = mpsc::channel(1);
        let (data_tx, _data_rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        let task_cancel = CancellationToken::new();
        let connection_cancel = CancellationToken::new();

        let forward = tokio::spawn(audio_forward_loop(
            audio_rx,
            peer_ctrl_rx,
            data_tx,
            ctrl_tx,
            task_cancel,
            connection_cancel.clone(),
        ));
        drop(peer_ctrl_tx);

        tokio::time::timeout(Duration::from_secs(1), forward)
            .await
            .expect("forwarder exits when its state-bearing queue closes")
            .expect("forwarder task completes cleanly");
        assert!(
            connection_cancel.is_cancelled(),
            "lost control state must tear down the WebSocket for a fresh roster"
        );
    }

    #[tokio::test]
    async fn audio_send_loop_sends_policy_close_when_community_is_deleted() {
        use futures_util::Sink;

        struct MockSink {
            messages: Arc<Mutex<Vec<WsMessage>>>,
        }

        impl Sink<WsMessage> for MockSink {
            type Error = std::io::Error;

            fn poll_ready(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn start_send(
                self: std::pin::Pin<&mut Self>,
                item: WsMessage,
            ) -> Result<(), Self::Error> {
                self.messages.lock().expect("mock sink poisoned").push(item);
                Ok(())
            }

            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn poll_close(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                self.poll_flush(cx)
            }
        }

        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let disconnect_reason = control.disconnect_reason();
        let registry = crate::state::CommunityConnectionRegistry::new();
        let community = nimino_core::CommunityId::from_uuid(Uuid::new_v4());
        let _guard = registry.register(Uuid::new_v4(), community, control);
        assert_eq!(registry.disconnect_community(community), 1);
        let messages = Arc::new(Mutex::new(Vec::new()));
        let sink = MockSink {
            messages: Arc::clone(&messages),
        };

        send_loop(sink, data_rx, ctrl_rx, cancel, disconnect_reason).await;

        let messages = messages.lock().expect("mock sink poisoned");
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            WsMessage::Close(Some(close)) => {
                assert_eq!(close.code, axum::extract::ws::close_code::POLICY);
                assert_eq!(close.reason.as_str(), "community deleted");
            }
            other => panic!("expected one 1008 deletion close, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn audio_websocket_parser_rejects_oversized_messages_before_handler_reads_them() {
        assert!(
            handler_receives_message_of_size(MAX_WEBSOCKET_MESSAGE_BYTES).await,
            "messages at the audio route limit should still reach the handler"
        );
        assert!(
            !handler_receives_message_of_size(MAX_WEBSOCKET_MESSAGE_BYTES + 1).await,
            "oversized messages must be rejected by the WebSocket parser before the handler sees them"
        );
    }
}
