//! Bounded node-local admission windows.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nimino_auth::{
    error::AuthError,
    rate_limit::{LimitType, RateLimitResult, RateLimiter},
};
use nimino_core::TenantContext;
use nostr::PublicKey;

#[derive(Clone, Copy)]
struct Window {
    started_at: Instant,
    count: u64,
}

/// Node-local fixed-window admission limiter.
#[derive(Default)]
pub struct LocalRateLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

impl LocalRateLimiter {
    /// Creates an empty limiter.
    pub fn new() -> Self {
        Self::default()
    }

    fn increment(
        &self,
        key: String,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        let now = Instant::now();
        let window_duration = Duration::from_secs(window_secs.max(1));
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| AuthError::Internal("rate-limit cache lock poisoned".into()))?;
        windows.retain(|_, window| now.duration_since(window.started_at) < window_duration);
        let window = windows.entry(key).or_insert(Window {
            started_at: now,
            count: 0,
        });
        if now.duration_since(window.started_at) >= window_duration {
            *window = Window {
                started_at: now,
                count: 0,
            };
        }
        window.count = window.count.saturating_add(1);
        let elapsed = now.duration_since(window.started_at).as_secs();
        let reset = window_secs.max(1).saturating_sub(elapsed);
        Ok(if window.count <= limit {
            RateLimitResult::allowed(window.count, limit, reset)
        } else {
            RateLimitResult::denied(window.count, limit, reset)
        })
    }
}

impl RateLimiter for LocalRateLimiter {
    async fn check_and_increment(
        &self,
        context: &TenantContext,
        pubkey: &PublicKey,
        limit_type: LimitType,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        self.increment(
            nimino_auth::rate_limit::rate_limit_key(context, pubkey, &limit_type),
            window_secs,
            limit,
        )
    }

    async fn check_ip_connection(
        &self,
        ip: &IpAddr,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        self.increment(
            nimino_auth::rate_limit::ip_rate_limit_key(ip),
            window_secs,
            limit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_window_denies_after_limit() {
        let limiter = LocalRateLimiter::new();
        assert!(limiter.increment("k".into(), 60, 1).unwrap().allowed);
        assert!(!limiter.increment("k".into(), 60, 1).unwrap().allowed);
    }
}
