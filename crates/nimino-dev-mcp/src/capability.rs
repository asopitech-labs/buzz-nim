use rmcp::ErrorData;
use serde::Serialize;
use std::collections::HashSet;

pub(crate) const AUDIT_CONTRACT: &str = "nimino.mcp-capability-audit/v1";
pub(crate) const CAPABILITY_DENIED: &str = "CAPABILITY_DENIED";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Capability {
    ProcessExec,
    FilesystemRead,
    FilesystemWrite,
    NetworkRead,
}

impl Capability {
    fn name(self) -> &'static str {
        match self {
            Self::ProcessExec => "process.exec",
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::NetworkRead => "network.read",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "process.exec" => Some(Self::ProcessExec),
            "filesystem.read" => Some(Self::FilesystemRead),
            "filesystem.write" => Some(Self::FilesystemWrite),
            "network.read" => Some(Self::NetworkRead),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CapabilityPolicy {
    granted: HashSet<Capability>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditEvent<'a> {
    contract: &'static str,
    tool: &'a str,
    capability: &'static str,
    outcome: &'static str,
    scope: &'a str,
}

impl CapabilityPolicy {
    pub(crate) fn from_env() -> std::io::Result<Self> {
        let configured = std::env::var("NIMINO_MCP_CAPABILITIES").ok();
        Self::parse(
            configured
                .as_deref()
                .unwrap_or("process.exec,filesystem.read,filesystem.write,network.read"),
        )
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    }

    fn parse(value: &str) -> Result<Self, String> {
        let mut granted = HashSet::new();
        for raw in value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let capability = Capability::parse(raw)
                .ok_or_else(|| format!("unknown NIMINO_MCP_CAPABILITIES value: {raw}"))?;
            granted.insert(capability);
        }
        Ok(Self { granted })
    }

    pub(crate) fn authorize(
        &self,
        tool: &str,
        capability: Capability,
        scope: &str,
    ) -> Result<(), ErrorData> {
        let allowed = self.granted.contains(&capability);
        let event = AuditEvent {
            contract: AUDIT_CONTRACT,
            tool,
            capability: capability.name(),
            outcome: if allowed { "authorized" } else { "denied" },
            scope,
        };
        let encoded = serde_json::to_string(&event).unwrap_or_else(|_| {
            format!(r#"{{"contract":"{AUDIT_CONTRACT}","outcome":"encoding-failed"}}"#)
        });
        tracing::info!(target: "nimino_dev_mcp::audit", audit = %encoded);
        if allowed {
            Ok(())
        } else {
            Err(ErrorData::invalid_request(
                format!(
                    "{CAPABILITY_DENIED}: tool {tool} requires {}",
                    capability.name()
                ),
                Some(serde_json::json!({
                    "contract": AUDIT_CONTRACT,
                    "capability": capability.name(),
                    "scope": scope,
                })),
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn only(capabilities: &[Capability]) -> Self {
        Self {
            granted: capabilities.iter().copied().collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn all_for_test() -> Self {
        Self::only(&[
            Capability::ProcessExec,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkRead,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_capability_is_rejected_and_missing_grant_denies() {
        assert!(CapabilityPolicy::parse("filesystem.read,unknown").is_err());
        let policy = CapabilityPolicy::only(&[Capability::FilesystemRead]);
        assert!(policy
            .authorize("read_file", Capability::FilesystemRead, "workspace")
            .is_ok());
        let denied = policy
            .authorize("shell", Capability::ProcessExec, "host-process")
            .expect_err("process capability must be denied");
        assert!(denied.message.contains(CAPABILITY_DENIED));
        assert_eq!(
            denied.data.expect("typed audit data")["contract"],
            AUDIT_CONTRACT
        );
    }
}
