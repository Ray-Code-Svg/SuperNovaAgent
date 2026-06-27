use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model_runtime::{ModelBudget, ModelProviderFailure};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter_ms: u64,
}

impl RetryPolicy {
    pub fn from_budget(budget: &ModelBudget) -> Self {
        Self {
            max_attempts: budget.max_retries.saturating_add(1).max(1),
            initial_backoff_ms: env_u64("SUPERNOVA_MODEL_RETRY_INITIAL_BACKOFF_MS", 500),
            max_backoff_ms: env_u64("SUPERNOVA_MODEL_RETRY_MAX_BACKOFF_MS", 4_000),
            jitter_ms: env_u64("SUPERNOVA_MODEL_RETRY_JITTER_MS", 250),
        }
    }

    pub fn should_retry(&self, attempt: u32, failure: &ModelProviderFailure) -> bool {
        failure.retryable && attempt < self.max_attempts
    }

    pub fn backoff_ms(&self, completed_attempt: u32) -> u64 {
        if self.initial_backoff_ms == 0 {
            return 0;
        }
        let exponent = completed_attempt.saturating_sub(1).min(20);
        let base = self
            .initial_backoff_ms
            .saturating_mul(2_u64.saturating_pow(exponent));
        base.min(self.max_backoff_ms).saturating_add(self.jitter())
    }

    pub fn sleep_before_retry(&self, completed_attempt: u32) {
        let backoff_ms = self.backoff_ms(completed_attempt);
        if backoff_ms > 0 {
            thread::sleep(Duration::from_millis(backoff_ms));
        }
    }

    fn jitter(&self) -> u64 {
        if self.jitter_ms == 0 {
            return 0;
        }
        now_ms() % self.jitter_ms
    }
}

pub fn retryable_transport_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "connection reset",
        "connection aborted",
        "connection refused",
        "connection closed",
        "broken pipe",
        "temporarily unavailable",
        "would block",
        "10060",
        "10054",
        "10053",
        "eof",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub fn classify_io_transport_retryable(kind: std::io::ErrorKind, message: &str) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
    ) || retryable_transport_message(message)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_millis() as u64
}
