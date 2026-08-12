use sha2::{Digest, Sha256};

use crate::UiRpcDbPool;

pub const RESUME_WINDOW: i64 = 5_000;
pub const RETAINED_CHANGES: i64 = 2 * RESUME_WINDOW;

const _: () = assert!(RETAINED_CHANGES > RESUME_WINDOW);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiRpcEnvironmentIdentity(String);

impl UiRpcEnvironmentIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn digest(&self) -> String {
        let digest = Sha256::digest(self.0.as_bytes());
        digest[..8].iter().fold(String::new(), |mut hex, byte| {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiRpcListingHead {
    pub identity: UiRpcEnvironmentIdentity,
    pub position: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRpcResume {
    From(i64),
    Rebuild,
}

pub fn resume(
    deployment: &UiRpcEnvironmentIdentity,
    database: &UiRpcEnvironmentIdentity,
    head: i64,
    after: i64,
) -> UiRpcResume {
    if database != deployment || after > head || head - after > RESUME_WINDOW {
        return UiRpcResume::Rebuild;
    }
    UiRpcResume::From(after)
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct UiRpcListingChange {
    pub position: i64,
    pub task_id: String,
}

pub async fn current_head(pool: &UiRpcDbPool) -> Result<UiRpcListingHead, sqlx::Error> {
    let (position, identity): (i64, String) = sqlx::query_as(
        r#"SELECT
               COALESCE((SELECT position FROM task_listing_position), 0),
               (SELECT system_identifier::text FROM pg_control_system())
                   || ':'
                   || (SELECT oid::text FROM pg_database
                       WHERE datname = current_database())"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(UiRpcListingHead {
        identity: UiRpcEnvironmentIdentity::new(identity),
        position,
    })
}

pub async fn changes_after(
    pool: &UiRpcDbPool,
    after: i64,
    limit: i64,
) -> Result<Vec<UiRpcListingChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, task_id
           FROM task_listing_changes
           WHERE position > $1
           ORDER BY position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}
