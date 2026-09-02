//! HMAC receipts for invite-bound join-policy acceptance.
//!
//! Invite codes themselves are durable `v2.` records owned by `nimino-db`.
//! This module only signs the short-lived proof that a browser accepted the
//! configured policy for one exact invite code.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Domain-separation label mixed into the HMAC key derivation.
const KEY_DERIVATION_LABEL: &[u8] = b"nimino-policy-acceptance-v1";

/// Why a code failed verification. Variants are deliberately coarse — the
/// HTTP layer maps all of them to a generic rejection so the endpoint does
/// not become an oracle for forging codes.
#[derive(Debug, PartialEq, Eq)]
pub enum InviteError {
    /// Structurally invalid (bad base64, bad JSON, wrong shape, too long).
    Malformed,
    /// MAC did not verify.
    BadSignature,
    /// Signature fine, but the expiry has passed.
    Expired,
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            InviteError::Malformed => "malformed invite code",
            InviteError::BadSignature => "invalid invite signature",
            InviteError::Expired => "invite code expired",
        };
        f.write_str(msg)
    }
}

/// Derive the invite HMAC key from the relay's signing secret.
///
/// `sha256(secret_key_bytes || label)` — the label domain-separates this use
/// from any other HMAC built on the same keypair.
pub fn derive_invite_key(relay_keys: &nostr::Keys) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(relay_keys.secret_key().as_secret_bytes());
    hasher.update(KEY_DERIVATION_LABEL);
    hasher.finalize().into()
}

fn sign_payload(key: &[u8; 32], payload_bytes: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(payload_bytes);
    mac.finalize().into_bytes().to_vec()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Short-lived proof that the browser accepted the configured join policy.
#[derive(Debug, Serialize, Deserialize)]
pub struct PolicyAcceptancePayload {
    /// SHA-256 of the invite code this acceptance is bound to.
    pub c: String,
    /// Configured policy version.
    pub v: String,
    /// Receipt expiry (unix seconds).
    pub e: u64,
}

/// Mint a relay-authenticated, invite-bound policy acceptance receipt.
pub fn mint_policy_acceptance(key: &[u8; 32], code: &str, version: &str) -> String {
    let payload = PolicyAcceptancePayload {
        c: hex::encode(Sha256::digest(code.as_bytes())),
        v: version.to_string(),
        e: now_unix() + 10 * 60,
    };
    let bytes = serde_json::to_vec(&payload).expect("policy acceptance serializes");
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&bytes),
        URL_SAFE_NO_PAD.encode(sign_payload(key, &bytes))
    )
}

/// Verify a policy receipt and bind it to the submitted invite and current policy.
pub fn verify_policy_acceptance(
    key: &[u8; 32],
    receipt: &str,
    code: &str,
    version: &str,
) -> Result<PolicyAcceptancePayload, InviteError> {
    if receipt.len() > 2048 {
        return Err(InviteError::Malformed);
    }
    let (payload, signature) = receipt.split_once('.').ok_or(InviteError::Malformed)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| InviteError::Malformed)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| InviteError::Malformed)?;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(&bytes);
    mac.verify_slice(&signature)
        .map_err(|_| InviteError::BadSignature)?;
    let payload: PolicyAcceptancePayload =
        serde_json::from_slice(&bytes).map_err(|_| InviteError::Malformed)?;
    if payload.e < now_unix() {
        return Err(InviteError::Expired);
    }
    let expected_code = hex::encode(Sha256::digest(code.as_bytes()));
    if payload.c != expected_code || payload.v != version {
        return Err(InviteError::Malformed);
    }
    Ok(payload)
}

#[cfg(test)]
mod policy_acceptance_tests {
    use super::*;

    #[test]
    fn policy_receipt_is_bound_to_invite_and_version() {
        let key = [7_u8; 32];
        let receipt = mint_policy_acceptance(&key, "invite-a", "v1");
        let payload =
            verify_policy_acceptance(&key, &receipt, "invite-a", "v1").expect("valid receipt");
        assert_eq!(payload.v, "v1");
        assert!(verify_policy_acceptance(&key, &receipt, "invite-b", "v1").is_err());
        assert!(verify_policy_acceptance(&key, &receipt, "invite-a", "v2").is_err());
        assert!(verify_policy_acceptance(&[8_u8; 32], &receipt, "invite-a", "v1").is_err());
    }
}
