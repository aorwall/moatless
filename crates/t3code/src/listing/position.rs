use sha2::{Digest, Sha256};

pub use moatless_core::change_feed::listing::{RESUME_WINDOW, RETAINED_CHANGES};

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
