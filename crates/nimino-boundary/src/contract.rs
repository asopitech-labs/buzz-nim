use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

/// Stable protocol name carried by every v1 frame.
pub const PROTOCOL_NAME: &str = "nimino.core.boundary";
/// The only accepted boundary version. No downgrade path exists.
pub const PROTOCOL_VERSION: u16 = 1;
/// SHA-256 of the checked-in v1 contract bundle.
pub const SCHEMA_HASH: &str = "3b799b4bf7bf4fb3720de2103cf37642c781ea3746de5a98ddd9f04a5293e233";
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
