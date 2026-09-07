//! Shared failed-login rate limiting.

use chrono::{DateTime, TimeDelta, Utc};
use sha2::{Digest, Sha256};

use moatless_core::db::DbPool;

const MAX_FAILURES: i32 = 10;
const WINDOW_SECS: i64 = 15 * 60;

/// PostgreSQL-backed limiter shared by every backend replica.
#[derive(Clone)]
pub struct LoginThrottle {
    db: DbPool,
}

impl LoginThrottle {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn is_locked(&self, bucket: &str, subject: &str) -> Result<bool, sqlx::Error> {
        Ok(super::throttle_dao::attempts(
            &self.db,
            bucket,
            subject_hash(subject),
            window_start(Utc::now()),
        )
        .await?
            >= MAX_FAILURES)
    }

    pub async fn record_failure(&self, bucket: &str, subject: &str) -> Result<(), sqlx::Error> {
        super::throttle_dao::record_failure(
            &self.db,
            bucket,
            subject_hash(subject),
            window_start(Utc::now()),
        )
        .await
    }

    pub async fn clear(&self, bucket: &str, subject: &str) -> Result<(), sqlx::Error> {
        super::throttle_dao::clear(&self.db, bucket, subject_hash(subject)).await
    }
}

fn subject_hash(subject: &str) -> Vec<u8> {
    Sha256::digest(subject.trim().to_lowercase().as_bytes()).to_vec()
}

fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let seconds = now.timestamp().div_euclid(WINDOW_SECS) * WINDOW_SECS;
    DateTime::from_timestamp(seconds, 0).unwrap_or(now - TimeDelta::seconds(WINDOW_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_are_case_and_whitespace_insensitive() {
        assert_eq!(
            subject_hash(" Alice@Example.com "),
            subject_hash("alice@example.com")
        );
    }

    #[test]
    fn fixed_windows_are_fifteen_minutes_wide() {
        let first = DateTime::parse_from_rfc3339("2026-08-12T10:01:00Z")
            .unwrap()
            .to_utc();
        let second = DateTime::parse_from_rfc3339("2026-08-12T10:14:59Z")
            .unwrap()
            .to_utc();
        assert_eq!(window_start(first), window_start(second));
    }
}
