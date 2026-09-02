//! Bounded process-local NIP-98 replay cache.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nimino_auth::{
    error::AuthError,
    nip98_replay::{Nip98ReplayGuard, DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS},
};
use nostr::EventId;

/// Node-local replay cache. Canonical replication owns cross-node replay facts.
#[derive(Default)]
pub struct LocalReplayGuard {
    seen: Mutex<HashMap<(String, EventId), Instant>>,
}

impl LocalReplayGuard {
    /// Creates an empty replay cache.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Nip98ReplayGuard for LocalReplayGuard {
    fn try_mark_in_scope<'a>(
        &'a self,
        scope: &'a str,
        event_id: &'a EventId,
        ttl_secs: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, AuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            let now = Instant::now();
            let mut seen = self
                .seen
                .lock()
                .map_err(|_| AuthError::Internal("replay cache lock poisoned".into()))?;
            seen.retain(|_, expires_at| *expires_at > now);
            let key = (scope.to_owned(), *event_id);
            if seen.contains_key(&key) {
                return Ok(false);
            }
            let ttl = ttl_secs.clamp(DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS);
            seen.insert(key, now + Duration::from_secs(ttl));
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    #[tokio::test]
    async fn first_mark_wins_per_scope() {
        let id = EventBuilder::new(Kind::HttpAuth, "")
            .sign_with_keys(&Keys::generate())
            .unwrap()
            .id;
        let guard = LocalReplayGuard::new();
        assert!(guard.try_mark_in_scope("a", &id, 1).await.unwrap());
        assert!(!guard.try_mark_in_scope("a", &id, 1).await.unwrap());
        assert!(guard.try_mark_in_scope("b", &id, 1).await.unwrap());
    }
}
