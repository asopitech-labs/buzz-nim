//! Typed content-addressed object synchronization policy boundary.

use serde::{Deserialize, Serialize};

/// Content-addressed object domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectKind {
    /// Media bytes.
    Media,
    /// Git pack bytes.
    GitPack,
    /// Git manifest bytes.
    GitManifest,
}

/// Object materialization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectFetchMode {
    /// Materialize every manifest object.
    Eager,
    /// Materialize requested and pinned objects only.
    Lazy,
}

/// Object policy effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectEffect {
    /// Reject without action.
    Reject,
    /// Required objects are present.
    Complete,
    /// Fetch returned objects.
    Fetch,
    /// Persist a pin.
    Pin,
    /// Remove a pin.
    Unpin,
    /// No change.
    Noop,
    /// Delete returned objects.
    Delete,
}

/// Stable object policy error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectPolicyError {
    /// No error.
    None,
    /// Manifest shape is invalid.
    InvalidManifest,
    /// Adapter did not verify the manifest digest.
    ManifestDigestMismatch,
    /// Community scope differs.
    ScopeMismatch,
    /// Lifecycle does not allow synchronization.
    LifecycleDenied,
    /// Caller cancelled synchronization.
    Cancelled,
    /// Object digest is invalid.
    DigestInvalid,
    /// Requested or pinned object is absent from the manifest.
    ObjectUnknown,
    /// Local or origin facts are invalid.
    LocalFactInvalid,
    /// Local bytes disagree with the manifest.
    LocalChecksumMismatch,
    /// No live origin has the object.
    MissingOrigin,
    /// Fetch bound is invalid.
    FetchLimitInvalid,
    /// Pin revision differs.
    PinRevisionConflict,
    /// Pin revision exhausted.
    PinRevisionOverflow,
    /// GC facts or bounds are invalid.
    GcInvalid,
}

/// One content-addressed object in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectDescriptor {
    /// Lowercase SHA-256 identity.
    pub digest: String,
    /// Exact byte length.
    pub size: u64,
    /// Object domain.
    pub kind: ObjectKind,
}

/// Versioned object manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectManifest {
    /// Community owner.
    pub community_id: String,
    /// Verified manifest digest.
    pub manifest_id: String,
    /// Monotonic manifest generation.
    pub generation: u64,
    /// Referenced objects.
    pub objects: Vec<ObjectDescriptor>,
}

/// Local byte and partial-transfer facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectLocalFact {
    /// Object digest.
    pub digest: String,
    /// Expected size.
    pub size: u64,
    /// Complete bytes exist.
    pub present: bool,
    /// Complete bytes were verified.
    pub verified: bool,
    /// Partial transfer exists.
    pub partial: bool,
    /// Durable partial offset.
    pub partial_offset: u64,
    /// First unreferenced epoch, if known.
    pub unreferenced_since_epoch: Option<u64>,
}

/// One authenticated possible origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectOriginFact {
    /// Chirps node identity.
    pub node_id: String,
    /// Whether the origin is currently eligible.
    pub available: bool,
    /// Sorted or unsorted advertised object digests.
    pub digests: Vec<String>,
}

/// Complete facts for an object synchronization plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectSyncRequest {
    /// Requested community.
    pub community_id: String,
    /// Verified manifest.
    pub manifest: ObjectManifest,
    /// Whether the adapter verified the manifest digest.
    pub manifest_digest_verified: bool,
    /// Whether cluster lifecycle permits sync.
    pub lifecycle_allows_sync: bool,
    /// Caller cancellation fact.
    pub cancelled: bool,
    /// Eager or lazy materialization.
    pub mode: ObjectFetchMode,
    /// Lazy requested digest or empty.
    pub requested_digest: String,
    /// Durable pin snapshot.
    pub pinned_digests: Vec<String>,
    /// Local byte facts.
    pub local_facts: Vec<ObjectLocalFact>,
    /// Authenticated origin facts.
    pub origins: Vec<ObjectOriginFact>,
    /// Maximum actions returned.
    pub max_fetches: u16,
}

/// One Nim-selected fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectFetchAction {
    /// Object digest.
    pub digest: String,
    /// Exact size.
    pub size: u64,
    /// Object domain.
    pub kind: ObjectKind,
    /// Selected Chirps origin.
    pub source_node_id: String,
    /// Exact durable resume offset.
    pub resume_offset: u64,
}

/// Bounded fetch plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectSyncPlan {
    /// Selected effect.
    pub effect: ObjectEffect,
    /// Stable error.
    pub error: ObjectPolicyError,
    /// Bounded ordered fetches.
    pub actions: Vec<ObjectFetchAction>,
}

/// Durable pin state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectPinState {
    /// Whether the state shape is valid.
    pub valid: bool,
    /// Community scope.
    pub community_id: String,
    /// Monotonic revision.
    pub revision: u64,
    /// Normalized pin set.
    pub digests: Vec<String>,
}

/// One pin mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectPinRequest {
    /// Community scope.
    pub community_id: String,
    /// Required revision.
    pub expected_revision: u64,
    /// Object digest.
    pub digest: String,
    /// True to pin, false to unpin.
    pub pin: bool,
}

/// Pin mutation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectPinDecision {
    /// Selected effect.
    pub effect: ObjectEffect,
    /// Stable error.
    pub error: ObjectPolicyError,
    /// Authoritative state.
    pub state: ObjectPinState,
}

/// Complete cross-community garbage-collection facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectGcRequest {
    /// Operator scope label.
    pub community_id: String,
    /// Current logical epoch.
    pub current_epoch: u64,
    /// Required unreferenced grace.
    pub grace_epochs: u64,
    /// Complete cross-community reference snapshot.
    pub referenced_digests: Vec<String>,
    /// Complete cross-community pin snapshot.
    pub pinned_digests: Vec<String>,
    /// Local objects.
    pub objects: Vec<ObjectLocalFact>,
    /// Maximum deletions returned.
    pub max_deletes: u16,
}

/// Bounded GC plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectGcPlan {
    /// Selected effect.
    pub effect: ObjectEffect,
    /// Stable error.
    pub error: ObjectPolicyError,
    /// Sorted bounded delete set.
    pub delete_digests: Vec<String>,
}

/// Typed object policy request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectPolicyRequest {
    /// Plan object materialization.
    Sync {
        /// Complete sync facts.
        request: ObjectSyncRequest,
    },
    /// Decide a pin mutation.
    Pin {
        /// Current pin state.
        state: ObjectPinState,
        /// Mutation request.
        request: ObjectPinRequest,
    },
    /// Plan garbage collection.
    Gc {
        /// Complete GC facts.
        request: ObjectGcRequest,
    },
}

/// Typed object policy result.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectPolicyResult {
    /// Synchronization plan.
    Sync {
        /// Nim-owned plan.
        result: ObjectSyncPlan,
    },
    /// Pin decision.
    Pin {
        /// Nim-owned state transition.
        result: ObjectPinDecision,
    },
    /// GC plan.
    Gc {
        /// Nim-owned plan.
        result: ObjectGcPlan,
    },
}
