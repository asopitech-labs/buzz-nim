#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Process-local presence and bounded security caches.
//!
//! Durable state and cross-node convergence belong to the Nimino store/sync
//! domain.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nimino_core::{CommunityId, TenantContext};
use nostr::PublicKey;
use tokio::sync::Mutex;

/// Node-local NIP-98 replay cache.
pub mod nip98_replay;
/// Node-local admission windows.
pub mod rate_limiter;

pub use nip98_replay::LocalReplayGuard;
pub use rate_limiter::LocalRateLimiter;
const PRESENCE_TTL: Duration = Duration::from_secs(180);

struct PresenceEntry {
    status: String,
    expires_at: Instant,
}

/// Process-local ephemeral state.
pub struct LocalDelivery {
    presence: Mutex<HashMap<(CommunityId, [u8; 32]), PresenceEntry>>,
}

impl LocalDelivery {
    /// Creates an empty delivery registry.
    pub fn new() -> Self {
        Self {
            presence: Mutex::new(HashMap::new()),
        }
    }

    /// Sets one node-local presence value with a fixed TTL.
    pub async fn set_presence(&self, context: &TenantContext, pubkey: &PublicKey, status: &str) {
        self.presence.lock().await.insert(
            (context.community(), pubkey.to_bytes()),
            PresenceEntry {
                status: status.to_owned(),
                expires_at: Instant::now() + PRESENCE_TTL,
            },
        );
    }

    /// Clears one node-local presence value.
    pub async fn clear_presence(&self, context: &TenantContext, pubkey: &PublicKey) {
        self.presence
            .lock()
            .await
            .remove(&(context.community(), pubkey.to_bytes()));
    }

    /// Gets one unexpired node-local presence value.
    pub async fn get_presence(
        &self,
        context: &TenantContext,
        pubkey: &PublicKey,
    ) -> Option<String> {
        self.get_presence_bulk(context, &[*pubkey])
            .await
            .remove(&pubkey.to_hex())
    }

    /// Gets unexpired node-local presence values keyed by public-key hex.
    pub async fn get_presence_bulk(
        &self,
        context: &TenantContext,
        pubkeys: &[PublicKey],
    ) -> HashMap<String, String> {
        let now = Instant::now();
        let mut presence = self.presence.lock().await;
        presence.retain(|_, entry| entry.expires_at > now);
        pubkeys
            .iter()
            .filter_map(|pubkey| {
                presence
                    .get(&(context.community(), pubkey.to_bytes()))
                    .map(|entry| (pubkey.to_hex(), entry.status.clone()))
            })
            .collect()
    }
}

impl Default for LocalDelivery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use uuid::Uuid;

    fn context() -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(1)), "example.test")
    }

    #[tokio::test]
    async fn presence_stays_process_local() {
        let delivery = LocalDelivery::new();
        let keys = Keys::generate();
        delivery
            .set_presence(&context(), &keys.public_key(), "online")
            .await;
        assert_eq!(
            delivery
                .get_presence(&context(), &keys.public_key())
                .await
                .as_deref(),
            Some("online")
        );
    }
}
