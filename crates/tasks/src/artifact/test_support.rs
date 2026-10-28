//! The two mocked stores an [`Artifacts`] reads, for the artifact tests.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use moatless_core::config::S3Config;

use super::dao::MockArtifactDao;
use super::operations::Artifacts;
use crate::collection::dao::MockRecordDao;
use crate::collection::test_support::records;

/// An Artifact store and a record store; expectations on the Artifact store
/// are set through `Deref`, on the record store through `records`.
#[derive(Default)]
pub(crate) struct TestStore {
    pub(crate) artifacts: MockArtifactDao,
    pub(crate) records: MockRecordDao,
}

impl Deref for TestStore {
    type Target = MockArtifactDao;

    fn deref(&self) -> &MockArtifactDao {
        &self.artifacts
    }
}

impl DerefMut for TestStore {
    fn deref_mut(&mut self) -> &mut MockArtifactDao {
        &mut self.artifacts
    }
}

/// `Artifacts` over `store` with no object storage, served from `public_url`
/// and signing page tokens with `signing_key`.
pub(crate) fn artifacts_over(
    store: TestStore,
    s3: Arc<S3Config>,
    public_url: &str,
    signing_key: Option<&str>,
) -> Artifacts {
    Artifacts::new(
        Arc::new(store.artifacts),
        None,
        s3,
        public_url,
        signing_key.map(str::to_string),
        Arc::new(records(store.records)),
    )
}

/// The S3 config of a deployment with no bucket.
pub(crate) fn no_bucket() -> Arc<S3Config> {
    Arc::new(S3Config {
        bucket_name: None,
        key_prefix: "uploads".to_string(),
        transcript_prefix: "transcripts".to_string(),
        workspace_prefix: "workspaces".to_string(),
        artifact_prefix: "artifacts".to_string(),
        region: "us-east-1".to_string(),
    })
}
