use std::{sync::Arc, time::Duration};

use nimino_boundary::{
    BoundaryClient, BoundaryError, BoundaryRequest, BoundaryResult, CallContext, ObjectEffect,
    ObjectGcPlan, ObjectGcRequest, ObjectPinDecision, ObjectPinRequest, ObjectPinState,
    ObjectPolicyError, ObjectPolicyRequest, ObjectPolicyResult, ObjectSyncRequest,
};
use nimino_chirps::{MeshClient, MeshRuntimeError, MeshSubscription, NodeId};
use serde::{Deserialize, Serialize};
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{InstallResult, LocalObjectStore, ObjectStoreError};

const WIRE_PREFIX: &[u8] = b"NIMINO-OBJECT/1\n";
const MESH_MESSAGE_BYTES: usize = 63 * 1024;
// Chirps' user frame is 64 KiB today; reserve room for the typed header.
const NETWORK_CHUNK_BYTES: usize = 60 * 1024;

/// Failures from Nim-planned object replication over Chirps.
#[derive(Debug, thiserror::Error)]
pub enum ObjectSyncError {
    /// The supervised Nim worker rejected or failed the policy call.
    #[error("object policy boundary failed: {0}")]
    Boundary(#[from] BoundaryError),
    /// Chirps rejected or stopped an authenticated transport operation.
    #[error("object transport failed: {0}")]
    Transport(#[from] MeshRuntimeError),
    /// The local content-addressed adapter failed.
    #[error("object store failed: {0}")]
    Store(#[from] ObjectStoreError),
    /// Nim rejected the supplied manifest, lifecycle, origin, pin, or GC facts.
    #[error("object policy rejected the operation: {0:?}")]
    Policy(ObjectPolicyError),
    /// The response variant did not match the requested policy operation.
    #[error("object policy returned an unexpected result")]
    UnexpectedPolicyResult,
    /// A frame violated the bounded object transfer protocol.
    #[error("invalid object transfer frame: {0}")]
    InvalidFrame(&'static str),
    /// The selected origin refused the exact transfer.
    #[error("object origin rejected the transfer: {0}")]
    PeerRejected(String),
    /// The selected origin did not respond before the configured deadline.
    #[error("object transfer timed out")]
    Timeout,
    /// The Nim-selected origin was not a canonical Chirps node identity.
    #[error("object origin is not a canonical Chirps node id")]
    InvalidNodeId,
    /// The responder task failed to join cleanly.
    #[error("object responder task failed: {0}")]
    Join(String),
}

/// Cloneable object policy and transfer facade.
#[derive(Clone)]
pub struct ObjectSyncClient {
    mesh: MeshClient,
    boundary: BoundaryClient,
    store: Arc<LocalObjectStore>,
    timeout: Duration,
    running: watch::Receiver<bool>,
}

impl ObjectSyncClient {
    /// Ask Nim for a bounded plan, then execute only its selected Chirps fetches.
    pub async fn sync(
        &self,
        request: ObjectSyncRequest,
    ) -> Result<Vec<InstallResult>, ObjectSyncError> {
        let result = self.policy(ObjectPolicyRequest::Sync { request }).await?;
        let ObjectPolicyResult::Sync { result } = result else {
            return Err(ObjectSyncError::UnexpectedPolicyResult);
        };
        if result.error != ObjectPolicyError::None || result.effect == ObjectEffect::Reject {
            return Err(ObjectSyncError::Policy(result.error));
        }
        let mut installed = Vec::with_capacity(result.actions.len());
        for action in result.actions {
            installed.push(self.fetch(action).await?);
        }
        Ok(installed)
    }

    /// True while the local responder still owns its Chirps subscription.
    pub fn is_running(&self) -> bool {
        *self.running.borrow()
    }

    /// Ask Nim for one revision-checked pin transition.
    pub async fn decide_pin(
        &self,
        state: ObjectPinState,
        request: ObjectPinRequest,
    ) -> Result<ObjectPinDecision, ObjectSyncError> {
        let result = self
            .policy(ObjectPolicyRequest::Pin { state, request })
            .await?;
        let ObjectPolicyResult::Pin { result } = result else {
            return Err(ObjectSyncError::UnexpectedPolicyResult);
        };
        Ok(result)
    }

    /// Ask Nim for a bounded deletion set from a complete reference/pin snapshot.
    pub async fn plan_gc(&self, request: ObjectGcRequest) -> Result<ObjectGcPlan, ObjectSyncError> {
        let result = self.policy(ObjectPolicyRequest::Gc { request }).await?;
        let ObjectPolicyResult::Gc { result } = result else {
            return Err(ObjectSyncError::UnexpectedPolicyResult);
        };
        Ok(result)
    }

    /// Ask Nim for a bounded deletion set and delete exactly those local objects.
    pub async fn gc(&self, request: ObjectGcRequest) -> Result<Vec<String>, ObjectSyncError> {
        let plan = self.plan_gc(request).await?;
        if plan.error != ObjectPolicyError::None || plan.effect == ObjectEffect::Reject {
            return Err(ObjectSyncError::Policy(plan.error));
        }
        for digest in &plan.delete_digests {
            self.store.delete(digest)?;
        }
        Ok(plan.delete_digests)
    }

    async fn policy(
        &self,
        request: ObjectPolicyRequest,
    ) -> Result<ObjectPolicyResult, ObjectSyncError> {
        let result = self
            .boundary
            .call(
                BoundaryRequest::object_policy(request),
                CallContext::with_timeout(self.timeout),
            )
            .await?;
        let BoundaryResult::ObjectPolicy(result) = result else {
            return Err(ObjectSyncError::UnexpectedPolicyResult);
        };
        Ok(result)
    }

    async fn fetch(
        &self,
        action: nimino_boundary::ObjectFetchAction,
    ) -> Result<InstallResult, ObjectSyncError> {
        let source = parse_node_id(&action.source_node_id)?;
        let transfer_id = action.digest.clone();
        let partial = self
            .store
            .begin_partial(&transfer_id, &action.digest, action.size)?;
        if partial.offset != action.resume_offset {
            return Err(ObjectSyncError::InvalidFrame(
                "Nim plan does not match durable partial offset",
            ));
        }
        let mut messages = self.mesh.subscribe();
        let mut offset = partial.offset;
        while offset < action.size {
            let request = WireHeader::Request {
                transfer_id: transfer_id.clone(),
                digest: action.digest.clone(),
                size: action.size,
                offset,
                max_bytes: NETWORK_CHUNK_BYTES as u32,
            };
            self.mesh.send(source, encode(&request, &[])?).await?;
            let deadline = tokio::time::sleep(self.timeout);
            tokio::pin!(deadline);
            let chunk = loop {
                tokio::select! {
                    _ = &mut deadline => return Err(ObjectSyncError::Timeout),
                    message = messages.recv() => {
                        let message = message?;
                        if message.from() != source {
                            continue;
                        }
                        let Some(frame) = decode(message.payload())? else {
                            continue;
                        };
                        match frame.header {
                            WireHeader::Chunk {
                                transfer_id: ref id,
                                ref digest,
                                size,
                                offset: frame_offset,
                            } if id == &transfer_id && digest == &action.digest => {
                                if size != action.size || frame_offset != offset || frame.body.is_empty() {
                                    return Err(ObjectSyncError::InvalidFrame("chunk identity or offset mismatch"));
                                }
                                break frame.body;
                            }
                            WireHeader::Reject {
                                transfer_id: ref id,
                                ref digest,
                                ref reason,
                            } if id == &transfer_id && digest == &action.digest => {
                                return Err(ObjectSyncError::PeerRejected(reason.clone()));
                            }
                            _ => continue,
                        }
                    }
                }
            };
            offset = self
                .store
                .append_partial(&transfer_id, &action.digest, action.size, offset, &chunk)?
                .offset;
        }
        match self
            .store
            .finish_partial(&transfer_id, &action.digest, action.size)
        {
            Ok(result) => Ok(result),
            Err(error @ ObjectStoreError::DigestMismatch { .. }) => {
                self.store
                    .abort_partial(&transfer_id, &action.digest, action.size)?;
                Err(error.into())
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// Owner of the bounded authenticated object responder task.
pub struct ObjectSyncRuntime {
    client: ObjectSyncClient,
    shutdown: CancellationToken,
    task: JoinHandle<Result<(), ObjectSyncError>>,
    running: watch::Receiver<bool>,
}

impl ObjectSyncRuntime {
    /// Start an object responder over an existing production Chirps mesh.
    pub fn start(
        mesh: MeshClient,
        boundary: BoundaryClient,
        store: Arc<LocalObjectStore>,
        timeout: Duration,
    ) -> Result<Self, ObjectSyncError> {
        if timeout.is_zero() {
            return Err(ObjectSyncError::InvalidFrame("timeout must be positive"));
        }
        let (running_tx, running) = watch::channel(true);
        let client = ObjectSyncClient {
            mesh: mesh.clone(),
            boundary,
            store: store.clone(),
            timeout,
            running: running.clone(),
        };
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let messages = mesh.subscribe();
        let task = tokio::spawn(async move {
            let result = serve(mesh, messages, store, task_shutdown).await;
            running_tx.send_replace(false);
            result
        });
        Ok(Self {
            client,
            shutdown,
            task,
            running,
        })
    }

    /// Return a cloneable request facade.
    pub fn client(&self) -> ObjectSyncClient {
        self.client.clone()
    }

    /// True while the responder still owns its Chirps subscription.
    pub fn is_running(&self) -> bool {
        *self.running.borrow()
    }

    /// Stop the responder and release its subscription.
    pub async fn shutdown(self) -> Result<(), ObjectSyncError> {
        self.shutdown.cancel();
        self.task
            .await
            .map_err(|error| ObjectSyncError::Join(error.to_string()))?
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum WireHeader {
    Request {
        transfer_id: String,
        digest: String,
        size: u64,
        offset: u64,
        max_bytes: u32,
    },
    Chunk {
        transfer_id: String,
        digest: String,
        size: u64,
        offset: u64,
    },
    Reject {
        transfer_id: String,
        digest: String,
        reason: String,
    },
}

struct DecodedFrame {
    header: WireHeader,
    body: Vec<u8>,
}

async fn serve(
    mesh: MeshClient,
    mut messages: MeshSubscription,
    store: Arc<LocalObjectStore>,
    shutdown: CancellationToken,
) -> Result<(), ObjectSyncError> {
    loop {
        let message = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            message = messages.recv() => message?,
        };
        let Some(frame) = (match decode(message.payload()) {
            Ok(frame) => frame,
            Err(_) => continue,
        }) else {
            continue;
        };
        let WireHeader::Request {
            transfer_id,
            digest,
            size,
            offset,
            max_bytes,
        } = frame.header
        else {
            continue;
        };
        if !frame.body.is_empty()
            || transfer_id.is_empty()
            || transfer_id.len() > 128
            || digest.len() != 64
        {
            continue;
        }
        let max_bytes = usize::try_from(max_bytes)
            .unwrap_or(usize::MAX)
            .min(NETWORK_CHUNK_BYTES);
        let response = match store.read_chunk(&digest, size, offset, max_bytes) {
            Ok(body) => encode(
                &WireHeader::Chunk {
                    transfer_id,
                    digest,
                    size,
                    offset,
                },
                &body,
            )?,
            Err(error) => encode(
                &WireHeader::Reject {
                    transfer_id,
                    digest,
                    reason: error.to_string().chars().take(256).collect(),
                },
                &[],
            )?,
        };
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            sent = mesh.send(message.from(), response) => {
                if matches!(sent, Err(MeshRuntimeError::Stopped)) {
                    return Err(MeshRuntimeError::Stopped.into());
                }
            }
        }
    }
}

fn encode(header: &WireHeader, body: &[u8]) -> Result<Vec<u8>, ObjectSyncError> {
    let header = serde_json::to_vec(header)
        .map_err(|_| ObjectSyncError::InvalidFrame("header cannot be encoded"))?;
    let header_len = u32::try_from(header.len())
        .map_err(|_| ObjectSyncError::InvalidFrame("header is too large"))?;
    let total = WIRE_PREFIX.len() + 4 + header.len() + body.len();
    if total > MESH_MESSAGE_BYTES {
        return Err(ObjectSyncError::InvalidFrame("frame exceeds Chirps limit"));
    }
    let mut result = Vec::with_capacity(total);
    result.extend_from_slice(WIRE_PREFIX);
    result.extend_from_slice(&header_len.to_be_bytes());
    result.extend_from_slice(&header);
    result.extend_from_slice(body);
    Ok(result)
}

fn decode(payload: &[u8]) -> Result<Option<DecodedFrame>, ObjectSyncError> {
    if !payload.starts_with(WIRE_PREFIX) {
        return Ok(None);
    }
    let length_start = WIRE_PREFIX.len();
    let header_start = length_start + 4;
    let length = payload
        .get(length_start..header_start)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_be_bytes)
        .ok_or(ObjectSyncError::InvalidFrame("missing header length"))?;
    let header_end = header_start
        .checked_add(length as usize)
        .filter(|end| *end <= payload.len())
        .ok_or(ObjectSyncError::InvalidFrame("invalid header length"))?;
    let header = serde_json::from_slice(&payload[header_start..header_end])
        .map_err(|_| ObjectSyncError::InvalidFrame("invalid header"))?;
    Ok(Some(DecodedFrame {
        header,
        body: payload[header_end..].to_vec(),
    }))
}

fn parse_node_id(value: &str) -> Result<NodeId, ObjectSyncError> {
    let bytes = hex::decode(value).map_err(|_| ObjectSyncError::InvalidNodeId)?;
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| ObjectSyncError::InvalidNodeId)?;
    if hex::encode(bytes) != value {
        return Err(ObjectSyncError::InvalidNodeId);
    }
    Ok(NodeId::from_bytes(bytes))
}
