use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

use crate::community::{CommunityPolicyRequest, CommunityPolicyResult};
use crate::dm::{DmPolicyRequest, DmPolicyResult};
use crate::membership::{MembershipPolicyRequest, MembershipPolicyResult};
use crate::moderation::{ModerationPolicyRequest, ModerationPolicyResult};
use crate::workflow::{WorkflowPolicyRequest, WorkflowPolicyResult};

/// Stable protocol name carried by every v1 frame.
pub const PROTOCOL_NAME: &str = "nimino.core.boundary";
/// The only accepted boundary version. No downgrade path exists.
pub const PROTOCOL_VERSION: u16 = 1;
/// SHA-256 of the checked-in v1 contract bundle.
pub const SCHEMA_HASH: &str = "7ee3f5ecd9696588c753c255b85279da5361f43ed6a52fee5afc43a592d75746";
/// Role required during the exact-match startup handshake.
pub const WORKER_ROLE: &str = "nimino-core";
/// Maximum JSON payload length accepted by the frame codec.
pub const MAX_FRAME_BYTES: usize = 1_048_576;
/// v1 executes one request at a time in each worker process.
pub const MAX_INFLIGHT: u16 = 1;

/// Retry classification supplied by Nimino for a typed remote failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    /// Repeating the operation cannot resolve the failure.
    Never,
    /// Only a caller that can prove idempotency may repeat the operation.
    IdempotentOnly,
    /// The caller must refresh its input facts before considering another call.
    AfterRefresh,
}

/// Stable worker-side error codes in the immutable v1 manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RemoteErrorCode {
    /// The request envelope or operation payload was invalid.
    InvalidRequest,
    /// The request used a protocol version other than v1.
    UnsupportedVersion,
    /// Host and worker contract facts did not match exactly.
    ContractMismatch,
    /// An operation was attempted before the startup handshake.
    HandshakeRequired,
    /// The operation is not implemented by this worker.
    UnknownOperation,
    /// A frame exceeded the fixed v1 byte limit.
    FrameTooLarge,
    /// The worker failed without a more specific stable classification.
    InternalError,
}

impl RemoteErrorCode {
    /// Returns the manifest spelling used on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::UnsupportedVersion => "UNSUPPORTED_VERSION",
            Self::ContractMismatch => "CONTRACT_MISMATCH",
            Self::HandshakeRequired => "HANDSHAKE_REQUIRED",
            Self::UnknownOperation => "UNKNOWN_OPERATION",
            Self::FrameTooLarge => "FRAME_TOO_LARGE",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }

    /// Returns the only retry disposition valid for this v1 code.
    pub const fn retry(self) -> RetryDisposition {
        match self {
            Self::HandshakeRequired => RetryDisposition::AfterRefresh,
            Self::InternalError => RetryDisposition::IdempotentOnly,
            Self::InvalidRequest
            | Self::UnsupportedVersion
            | Self::ContractMismatch
            | Self::UnknownOperation
            | Self::FrameTooLarge => RetryDisposition::Never,
        }
    }
}

/// Stable failure returned by the Nim worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryFault {
    /// Machine-readable code from the versioned error manifest.
    pub code: RemoteErrorCode,
    /// Bounded, human-readable diagnostic without an implementation backtrace.
    pub message: String,
    /// Explicit rule governing whether another attempt may be considered.
    pub retry: RetryDisposition,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryFaultWire {
    code: RemoteErrorCode,
    message: String,
    retry: RetryDisposition,
}

impl<'de> Deserialize<'de> for BoundaryFault {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BoundaryFaultWire::deserialize(deserializer)?;
        if wire.retry != wire.code.retry() {
            return Err(D::Error::custom(
                "remote error code and retry disposition do not match",
            ));
        }
        if wire.message.chars().count() > 1_024 {
            return Err(D::Error::custom("remote error message is too long"));
        }
        Ok(Self {
            code: wire.code,
            message: wire.message,
            retry: wire.retry,
        })
    }
}

/// Payload for the typed diagnostic echo operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EchoPayload {
    /// JSON data returned unchanged by the diagnostic worker operation.
    pub data: Value,
}

/// Verified version facts used by Nimino's replacement ordering policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventVersion {
    /// Signed event timestamp.
    pub created_at: i64,
    /// Canonical event identifier.
    pub event_id: String,
}

/// Existing thread metadata supplied by the storage adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadMetadataFacts {
    /// Canonical thread root identifier.
    pub root_id: String,
    /// Existing reply depth.
    pub depth: i32,
}

/// Verified parent facts used to derive a new reply plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadParentFacts {
    /// Parent event identifier.
    pub event_id: String,
    /// Parent event timestamp.
    pub created_at: i64,
    /// Parent community channel identifier.
    pub channel_id: String,
    /// Parent Nostr tags, used only when indexed metadata is absent.
    pub tags: Vec<Vec<String>>,
    /// Indexed ancestry, when available.
    pub metadata: Option<ThreadMetadataFacts>,
}

/// Facts required to derive NIP-10 ancestry and counter changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadRequest {
    /// Incoming event identifier.
    pub event_id: String,
    /// Incoming event timestamp.
    pub created_at: i64,
    /// Incoming event channel identifier.
    pub channel_id: String,
    /// Incoming Nostr tags.
    pub tags: Vec<Vec<String>>,
    /// Verified parent facts, if a reply marker was present.
    pub parent: Option<ThreadParentFacts>,
    /// Verified root timestamp, when it differs from the parent timestamp.
    pub root_created_at: Option<i64>,
}

/// Verified event target used by NIP-09 deletion policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeletionTargetFacts {
    /// Target event identifier.
    pub event_id: String,
    /// Effective target author.
    pub author: String,
    /// Target event timestamp.
    pub created_at: i64,
    /// Whether the target is currently live.
    pub active: bool,
    /// Parent identifier for reply-counter repair.
    pub parent_id: Option<String>,
    /// Root identifier for descendant-counter repair.
    pub root_id: Option<String>,
}

/// Facts required to decide a standard NIP-09 deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeletionRequest {
    /// Effective deletion author.
    pub actor: String,
    /// Deletion event timestamp.
    pub created_at: i64,
    /// Referenced event identifiers.
    pub e_targets: Vec<String>,
    /// Referenced addressable coordinates.
    pub a_targets: Vec<String>,
    /// Verified target event facts for an event-id deletion.
    pub target: Option<DeletionTargetFacts>,
}

/// Facts required to decide a NIP-25 reaction mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReactionRequest {
    /// Whether the referenced event exists in this community.
    pub target_exists: bool,
    /// Whether the same active actor/target/emoji tuple already exists.
    pub active_duplicate: bool,
    /// Reaction content; empty content means `+`.
    pub content: String,
    /// Nostr tags used to verify long custom emoji shortcodes.
    pub tags: Vec<Vec<String>>,
}

/// Typed event-policy decision requested from the Nimino core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EventPolicyRequest {
    /// Classify a registered kind and its parameterized key shape.
    Classify {
        /// Nostr event kind.
        kind: u32,
        /// Number of `d` tags.
        d_tag_count: u16,
        /// UTF-8 byte length of the sole `d` tag, or zero when absent.
        d_tag_len: u16,
    },
    /// Decide whether an event becomes the replaceable head.
    Replacement {
        /// Incoming signed event version.
        incoming: EventVersion,
        /// Current stored head, when one exists.
        current: Option<EventVersion>,
    },
    /// Derive NIP-10 ancestry and counter mutations.
    Thread {
        /// Verified incoming and parent facts.
        request: ThreadRequest,
    },
    /// Decide a NIP-09 event or coordinate deletion.
    Deletion {
        /// Verified deletion and target facts.
        request: DeletionRequest,
    },
    /// Decide a NIP-25 reaction insert or duplicate.
    Reaction {
        /// Verified reaction target and emoji facts.
        request: ReactionRequest,
    },
}

/// Stable event-policy validation failures returned by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPolicyError {
    /// The decision is valid.
    None,
    /// The kind is outside the Nimino v1 registry.
    UnsupportedKind,
    /// NIP-42 authentication must not enter event storage.
    AuthNotStorable,
    /// A parameterized event omitted its `d` tag.
    DTagRequired,
    /// A parameterized event supplied more than one `d` tag.
    DTagCardinality,
    /// The `d` tag exceeds the fixed v1 limit.
    DTagTooLong,
    /// A reply marker had no verified parent facts.
    ThreadParentMissing,
    /// The reply marker and supplied parent disagree.
    ThreadParentMismatch,
    /// The reply and parent belong to different channels.
    ThreadChannelMismatch,
    /// The client root disagrees with verified ancestry.
    ThreadRootMismatch,
    /// The derived thread depth exceeds the fixed v1 limit.
    ThreadDepthExceeded,
    /// A deletion did not name exactly one event or coordinate.
    DeleteTargetCardinality,
    /// The referenced event was missing.
    DeleteTargetMissing,
    /// The referenced identifier and supplied facts disagree.
    DeleteTargetMismatch,
    /// The actor does not own the target.
    DeleteAuthorMismatch,
    /// An addressable coordinate is malformed or foreign-owned.
    DeleteCoordinateInvalid,
    /// The reaction target was missing.
    ReactionTargetMissing,
    /// Reaction content violates the NIP-25/NIP-30 bounds.
    ReactionEmojiInvalid,
}

/// Storage disposition assigned to an accepted kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventDisposition {
    /// The event is rejected.
    Rejected,
    /// The event is stored normally.
    Stored,
    /// The event is transient and is not stored.
    Ephemeral,
    /// The event replaces the author's head for its kind.
    Replaceable,
    /// The event replaces the author's `(kind, d)` head.
    Parameterized,
}

/// Deterministic action for a replaceable event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplacementAction {
    /// Insert when no head exists.
    Insert,
    /// Replace the current head.
    Replace,
    /// The exact event is already the head.
    Duplicate,
    /// A newer or tie-winning head already exists.
    Stale,
}

/// Derived NIP-10 ancestry and materialized-counter mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadPlan {
    /// Canonical root event identifier.
    pub root_id: String,
    /// Direct parent event identifier.
    pub parent_id: String,
    /// Verified root timestamp.
    pub root_created_at: i64,
    /// Verified parent timestamp.
    pub parent_created_at: i64,
    /// New reply depth.
    pub depth: i32,
    /// Whether the exact `broadcast=1` tag is present.
    pub broadcast: bool,
    /// Delta for the direct parent's reply count.
    pub parent_reply_delta: i32,
    /// Delta for the root's descendant count.
    pub root_descendant_delta: i32,
}

/// Mutation selected for a deletion request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionAction {
    /// No mutation is allowed.
    Reject,
    /// The target is already absent from live state.
    Noop,
    /// Soft-delete the named event.
    DeleteEvent,
    /// Soft-delete the named addressable coordinate.
    DeleteCoordinate,
    /// Preserve a replacement newer than the tombstone.
    KeepNewer,
}

/// Mutation selected for a reaction request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionAction {
    /// No mutation is allowed.
    Reject,
    /// The active reaction already exists.
    Duplicate,
    /// Insert the canonical reaction.
    Insert,
}

/// Typed result of a Nimino event-policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EventPolicyResult {
    /// Kind classification result.
    Classify {
        /// Storage disposition.
        disposition: EventDisposition,
        /// Validation outcome.
        error: EventPolicyError,
    },
    /// Replacement ordering result.
    Replacement {
        /// Selected replacement action.
        action: ReplacementAction,
    },
    /// Thread ancestry result.
    Thread {
        /// Validation outcome.
        error: EventPolicyError,
        /// Derived plan, absent when rejected or top-level.
        plan: Option<ThreadPlan>,
    },
    /// Deletion result.
    Deletion {
        /// Validation outcome.
        error: EventPolicyError,
        /// Selected deletion action.
        action: DeletionAction,
        /// Delta for the direct parent's reply count.
        parent_reply_delta: i32,
        /// Delta for the root's descendant count.
        root_descendant_delta: i32,
    },
    /// Reaction result.
    Reaction {
        /// Validation outcome.
        error: EventPolicyError,
        /// Selected reaction action.
        action: ReactionAction,
        /// Canonical reaction content.
        emoji: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HelloPayload {
    pub(crate) schema_hash: String,
    pub(crate) worker_role: String,
    pub(crate) max_frame_bytes: usize,
    pub(crate) max_inflight: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Exact startup facts returned by a ready Nimino worker.
pub struct ReadyPayload {
    /// Semantic worker implementation version.
    pub worker_version: String,
    /// Exact boundary protocol version accepted by the worker.
    pub protocol_version: u16,
    /// SHA-256 of the immutable v1 schema bundle.
    pub schema_hash: String,
    /// Exact process role accepted by the host.
    pub worker_role: String,
    /// Maximum JSON payload bytes accepted by the worker.
    pub max_frame_bytes: usize,
    /// Maximum concurrent calls accepted by this worker.
    pub max_inflight: u16,
    /// Typed operation names advertised by the worker.
    pub capabilities: Vec<String>,
}

/// Typed success union discriminated by the response operation tag.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryResult {
    /// Exact startup handshake result.
    Ready(ReadyPayload),
    /// Diagnostic echo result.
    Echo(EchoPayload),
    /// Event acceptance and message mutation decision owned by Nimino.
    EventPolicy(EventPolicyResult),
    /// Community lifecycle and tenant isolation decision owned by Nimino.
    CommunityPolicy(CommunityPolicyResult),
    /// Channel, relay-roster, invite, and ownership decision owned by Nimino.
    MembershipPolicy(MembershipPolicyResult),
    /// Direct-message mutation and access decision owned by Nimino.
    DmPolicy(DmPolicyResult),
    /// Report, restriction, and resolution decision owned by Nimino.
    ModerationPolicy(ModerationPolicyResult),
    /// Definition, condition, planning, and transition decision owned by Nimino.
    WorkflowPolicy(WorkflowPolicyResult),
    /// Test-only operation result; never present in a production build.
    #[cfg(feature = "test-hooks")]
    Test(Value),
}

#[cfg(feature = "test-hooks")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SleepPayload {
    pub(crate) milliseconds: u64,
}

#[cfg(feature = "test-hooks")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmptyPayload {}

/// Typed operation union for the v1 request envelope.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BoundaryOperation {
    /// Exact-match process startup handshake.
    Hello(HelloPayload),
    /// Transport diagnostic that returns its typed payload unchanged.
    Echo(EchoPayload),
    /// Event acceptance and message mutation policy.
    EventPolicy(EventPolicyRequest),
    /// Community lifecycle and tenant isolation policy.
    CommunityPolicy(CommunityPolicyRequest),
    /// Channel, relay-roster, invite, and ownership policy.
    MembershipPolicy(MembershipPolicyRequest),
    /// Direct-message mutation and access policy.
    DmPolicy(DmPolicyRequest),
    /// Report, restriction, and resolution policy.
    ModerationPolicy(ModerationPolicyRequest),
    /// Workflow definition, condition, planning, and transition policy.
    WorkflowPolicy(WorkflowPolicyRequest),
    /// Deterministic blocking operation available only to boundary tests.
    #[cfg(feature = "test-hooks")]
    TestSleep(SleepPayload),
    /// Deterministic process exit available only to boundary tests.
    #[cfg(feature = "test-hooks")]
    TestCrash(EmptyPayload),
    /// Deterministic typed remote failure available only to boundary tests.
    #[cfg(feature = "test-hooks")]
    TestRemoteFailure(EmptyPayload),
    /// Writes non-frame bytes to standard output for corruption tests.
    #[cfg(feature = "test-hooks")]
    TestGarbage(EmptyPayload),
    /// Writes a framed but invalid JSON response for corruption tests.
    #[cfg(feature = "test-hooks")]
    TestMalformed(EmptyPayload),
    /// Responds with the wrong request identifier for correlation tests.
    #[cfg(feature = "test-hooks")]
    TestWrongId(EmptyPayload),
    /// Returns the worker process identifier for reap verification.
    #[cfg(feature = "test-hooks")]
    TestPid(EmptyPayload),
}

impl BoundaryOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::Hello(_) => "system.hello",
            Self::Echo(_) => "boundary.echo",
            Self::EventPolicy(_) => "domain.event.policy",
            Self::CommunityPolicy(_) => "domain.community.policy",
            Self::MembershipPolicy(_) => "domain.membership.policy",
            Self::DmPolicy(_) => "domain.dm.policy",
            Self::ModerationPolicy(_) => "domain.moderation.policy",
            Self::WorkflowPolicy(_) => "domain.workflow.policy",
            #[cfg(feature = "test-hooks")]
            Self::TestSleep(_) => "boundary.test.sleep",
            #[cfg(feature = "test-hooks")]
            Self::TestCrash(_) => "boundary.test.crash",
            #[cfg(feature = "test-hooks")]
            Self::TestRemoteFailure(_) => "boundary.test.remote_failure",
            #[cfg(feature = "test-hooks")]
            Self::TestGarbage(_) => "boundary.test.garbage",
            #[cfg(feature = "test-hooks")]
            Self::TestMalformed(_) => "boundary.test.malformed",
            #[cfg(feature = "test-hooks")]
            Self::TestWrongId(_) => "boundary.test.wrong_id",
            #[cfg(feature = "test-hooks")]
            Self::TestPid(_) => "boundary.test.pid",
        }
    }

    fn payload(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Hello(payload) => serde_json::to_value(payload),
            Self::Echo(payload) => serde_json::to_value(payload),
            Self::EventPolicy(payload) => serde_json::to_value(payload),
            Self::CommunityPolicy(payload) => serde_json::to_value(payload),
            Self::MembershipPolicy(payload) => serde_json::to_value(payload),
            Self::DmPolicy(payload) => serde_json::to_value(payload),
            Self::ModerationPolicy(payload) => serde_json::to_value(payload),
            Self::WorkflowPolicy(payload) => serde_json::to_value(payload),
            #[cfg(feature = "test-hooks")]
            Self::TestSleep(payload) => serde_json::to_value(payload),
            #[cfg(feature = "test-hooks")]
            Self::TestCrash(payload)
            | Self::TestRemoteFailure(payload)
            | Self::TestGarbage(payload)
            | Self::TestMalformed(payload)
            | Self::TestWrongId(payload)
            | Self::TestPid(payload) => serde_json::to_value(payload),
        }
    }

    fn decode(name: &str, payload: Value) -> Result<Self, serde_json::Error> {
        match name {
            "system.hello" => serde_json::from_value(payload).map(Self::Hello),
            "boundary.echo" => serde_json::from_value(payload).map(Self::Echo),
            "domain.event.policy" => serde_json::from_value(payload).map(Self::EventPolicy),
            "domain.community.policy" => serde_json::from_value(payload).map(Self::CommunityPolicy),
            "domain.membership.policy" => {
                serde_json::from_value(payload).map(Self::MembershipPolicy)
            }
            "domain.dm.policy" => serde_json::from_value(payload).map(Self::DmPolicy),
            "domain.moderation.policy" => {
                serde_json::from_value(payload).map(Self::ModerationPolicy)
            }
            "domain.workflow.policy" => serde_json::from_value(payload).map(Self::WorkflowPolicy),
            #[cfg(feature = "test-hooks")]
            "boundary.test.sleep" => serde_json::from_value(payload).map(Self::TestSleep),
            #[cfg(feature = "test-hooks")]
            "boundary.test.crash" => serde_json::from_value(payload).map(Self::TestCrash),
            #[cfg(feature = "test-hooks")]
            "boundary.test.remote_failure" => {
                serde_json::from_value(payload).map(Self::TestRemoteFailure)
            }
            #[cfg(feature = "test-hooks")]
            "boundary.test.garbage" => serde_json::from_value(payload).map(Self::TestGarbage),
            #[cfg(feature = "test-hooks")]
            "boundary.test.malformed" => serde_json::from_value(payload).map(Self::TestMalformed),
            #[cfg(feature = "test-hooks")]
            "boundary.test.wrong_id" => serde_json::from_value(payload).map(Self::TestWrongId),
            #[cfg(feature = "test-hooks")]
            "boundary.test.pid" => serde_json::from_value(payload).map(Self::TestPid),
            _ => Err(<serde_json::Error as serde::de::Error>::custom(format!(
                "unknown boundary operation: {name}"
            ))),
        }
    }
}

/// A versioned request with a typed operation payload.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryRequest {
    protocol: String,
    version: u16,
    request_id: String,
    operation: BoundaryOperation,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BoundaryRequestWireRef<'a> {
    protocol: &'a str,
    version: u16,
    request_id: &'a str,
    operation: &'a str,
    payload: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryRequestWire {
    protocol: String,
    version: u16,
    request_id: String,
    operation: String,
    payload: Value,
}

impl Serialize for BoundaryRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let payload = self
            .operation
            .payload()
            .map_err(serde::ser::Error::custom)?;
        BoundaryRequestWireRef {
            protocol: &self.protocol,
            version: self.version,
            request_id: &self.request_id,
            operation: self.operation.name(),
            payload,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BoundaryRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BoundaryRequestWire::deserialize(deserializer)?;
        if wire.protocol != PROTOCOL_NAME {
            return Err(D::Error::custom("unsupported boundary protocol name"));
        }
        if wire.version != PROTOCOL_VERSION {
            return Err(D::Error::custom("unsupported boundary protocol version"));
        }
        if wire.request_id.is_empty() || wire.request_id.chars().count() > 128 {
            return Err(D::Error::custom("invalid boundary request identifier"));
        }
        let operation =
            BoundaryOperation::decode(&wire.operation, wire.payload).map_err(D::Error::custom)?;
        Ok(Self {
            protocol: wire.protocol,
            version: wire.version,
            request_id: wire.request_id,
            operation,
        })
    }
}

impl BoundaryRequest {
    pub(crate) fn hello() -> Self {
        Self::from_operation(BoundaryOperation::Hello(HelloPayload {
            schema_hash: SCHEMA_HASH.to_owned(),
            worker_role: WORKER_ROLE.to_owned(),
            max_frame_bytes: MAX_FRAME_BYTES,
            max_inflight: MAX_INFLIGHT,
        }))
    }

    /// Constructs the typed diagnostic echo request.
    pub fn echo(data: Value) -> Self {
        Self::from_operation(BoundaryOperation::Echo(EchoPayload { data }))
    }

    /// Constructs a typed event acceptance or message mutation decision.
    pub fn event_policy(request: EventPolicyRequest) -> Self {
        Self::from_operation(BoundaryOperation::EventPolicy(request))
    }

    /// Constructs a typed community lifecycle or tenant isolation decision.
    pub fn community_policy(request: CommunityPolicyRequest) -> Self {
        Self::from_operation(BoundaryOperation::CommunityPolicy(request))
    }

    /// Constructs a typed channel, relay-roster, invite, or ownership decision.
    pub fn membership_policy(request: MembershipPolicyRequest) -> Self {
        Self::from_operation(BoundaryOperation::MembershipPolicy(request))
    }

    /// Constructs a typed direct-message mutation or access decision.
    pub fn dm_policy(request: DmPolicyRequest) -> Self {
        Self::from_operation(BoundaryOperation::DmPolicy(request))
    }

    /// Constructs a typed report, restriction, or resolution decision.
    pub fn moderation_policy(request: ModerationPolicyRequest) -> Self {
        Self::from_operation(BoundaryOperation::ModerationPolicy(request))
    }

    /// Constructs a typed workflow policy decision.
    pub fn workflow_policy(request: WorkflowPolicyRequest) -> Self {
        Self::from_operation(BoundaryOperation::WorkflowPolicy(request))
    }

    /// Returns the exact protocol identifier carried by this request.
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// Returns the exact protocol version carried by this request.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Returns the host-generated request correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the stable tag of the typed operation variant.
    pub fn operation_name(&self) -> &str {
        self.operation.name()
    }

    /// Returns the diagnostic echo data when this is an echo request.
    pub fn echo_data(&self) -> Option<&Value> {
        match &self.operation {
            BoundaryOperation::Echo(payload) => Some(&payload.data),
            _ => None,
        }
    }

    /// Constructs a deterministic blocking request for cross-language tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_sleep(milliseconds: u64) -> Self {
        Self::from_operation(BoundaryOperation::TestSleep(SleepPayload { milliseconds }))
    }

    /// Constructs a deterministic worker-crash request for lifecycle tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_crash() -> Self {
        Self::from_operation(BoundaryOperation::TestCrash(EmptyPayload {}))
    }

    /// Constructs a deterministic typed remote failure for contract tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_remote_failure() -> Self {
        Self::from_operation(BoundaryOperation::TestRemoteFailure(EmptyPayload {}))
    }

    /// Constructs a deterministic stdout-corruption request for framing tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_garbage() -> Self {
        Self::from_operation(BoundaryOperation::TestGarbage(EmptyPayload {}))
    }

    /// Constructs a deterministic malformed-response request for codec tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_malformed() -> Self {
        Self::from_operation(BoundaryOperation::TestMalformed(EmptyPayload {}))
    }

    /// Constructs a deterministic correlation mismatch request.
    #[cfg(feature = "test-hooks")]
    pub fn test_wrong_id() -> Self {
        Self::from_operation(BoundaryOperation::TestWrongId(EmptyPayload {}))
    }

    /// Constructs a deterministic process-identifier request for reap tests.
    #[cfg(feature = "test-hooks")]
    pub fn test_pid() -> Self {
        Self::from_operation(BoundaryOperation::TestPid(EmptyPayload {}))
    }

    fn from_operation(operation: BoundaryOperation) -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryResponseWire {
    protocol: String,
    version: u16,
    request_id: String,
    operation: String,
    status: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<BoundaryFault>,
}

#[derive(Debug, Clone, PartialEq)]
enum BoundaryResponseBody {
    Success(BoundaryResult),
    Failure(BoundaryFault),
}

/// Strictly decoded response from the Nim worker.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryResponse {
    protocol: String,
    version: u16,
    request_id: String,
    operation: String,
    body: BoundaryResponseBody,
}

impl<'de> Deserialize<'de> for BoundaryResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BoundaryResponseWire::deserialize(deserializer)?;
        if wire.protocol != PROTOCOL_NAME {
            return Err(D::Error::custom("unsupported boundary protocol name"));
        }
        if wire.version != PROTOCOL_VERSION {
            return Err(D::Error::custom("unsupported boundary protocol version"));
        }
        if wire.request_id.is_empty() || wire.request_id.chars().count() > 128 {
            return Err(D::Error::custom("invalid boundary response identifier"));
        }
        if wire.operation.is_empty() || wire.operation.chars().count() > 128 {
            return Err(D::Error::custom("invalid boundary response operation"));
        }
        let body = match (wire.status.as_str(), wire.result, wire.error) {
            ("ok", Some(result), None) => {
                let result = match wire.operation.as_str() {
                    "system.hello" => serde_json::from_value(result)
                        .map(BoundaryResult::Ready)
                        .map_err(D::Error::custom)?,
                    "boundary.echo" => serde_json::from_value(result)
                        .map(BoundaryResult::Echo)
                        .map_err(D::Error::custom)?,
                    "domain.event.policy" => serde_json::from_value(result)
                        .map(BoundaryResult::EventPolicy)
                        .map_err(D::Error::custom)?,
                    "domain.community.policy" => serde_json::from_value(result)
                        .map(BoundaryResult::CommunityPolicy)
                        .map_err(D::Error::custom)?,
                    "domain.membership.policy" => serde_json::from_value(result)
                        .map(BoundaryResult::MembershipPolicy)
                        .map_err(D::Error::custom)?,
                    "domain.dm.policy" => serde_json::from_value(result)
                        .map(BoundaryResult::DmPolicy)
                        .map_err(D::Error::custom)?,
                    "domain.moderation.policy" => serde_json::from_value(result)
                        .map(BoundaryResult::ModerationPolicy)
                        .map_err(D::Error::custom)?,
                    "domain.workflow.policy" => serde_json::from_value(result)
                        .map(BoundaryResult::WorkflowPolicy)
                        .map_err(D::Error::custom)?,
                    #[cfg(feature = "test-hooks")]
                    name if name.starts_with("boundary.test.") => BoundaryResult::Test(result),
                    _ => return Err(D::Error::custom("unknown success response operation")),
                };
                BoundaryResponseBody::Success(result)
            }
            ("error", None, Some(error)) => BoundaryResponseBody::Failure(error),
            _ => return Err(D::Error::custom("response status/body shape is invalid")),
        };
        Ok(Self {
            protocol: wire.protocol,
            version: wire.version,
            request_id: wire.request_id,
            operation: wire.operation,
            body,
        })
    }
}

impl BoundaryResponse {
    pub(crate) fn protocol(&self) -> &str {
        &self.protocol
    }

    pub(crate) fn version(&self) -> u16 {
        self.version
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }

    /// Converts the typed response body into its success value or remote fault.
    pub fn into_result(self) -> Result<BoundaryResult, BoundaryFault> {
        match self.body {
            BoundaryResponseBody::Success(result) => Ok(result),
            BoundaryResponseBody::Failure(error) => Err(error),
        }
    }
}
