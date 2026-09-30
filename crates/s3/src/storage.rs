use std::time::Duration;

use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{Credentials, Region, SharedCredentialsProvider};

/// How many times one request is attempted before the call carrying it fails.
/// Above `RetryConfig::standard()`'s own 3: a restic snapshot is hundreds of
/// requests and fails on the first one that runs out of attempts.
const MAX_ATTEMPTS: u32 = 5;

/// How long one attempt may spend reaching the endpoint.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Static keys a client signs with, such as a scoped STS session's.
pub struct S3Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl S3Credentials {
    fn signing(self) -> Credentials {
        Credentials::new(
            self.access_key_id,
            self.secret_access_key,
            self.session_token,
            None,
            "moatless",
        )
    }
}

/// A handle on S3: every object operation and presign in this crate takes one.
#[derive(Clone, Debug)]
pub struct S3Storage {
    pub(crate) client: aws_sdk_s3::Client,
    /// Present when the credentials can expire, so a presign can cap its TTL.
    credentials_provider: Option<SharedCredentialsProvider>,
}

impl S3Storage {
    /// A client in `region` on the default credential chain: environment,
    /// web identity (IRSA), then profile.
    #[cfg(feature = "default-chain")]
    pub async fn from_default_chain(region: impl Into<String>) -> Self {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(Region::new(region.into()))
            .load()
            .await;
        Self::new(
            aws_sdk_s3::Client::new(&config),
            config.credentials_provider(),
        )
    }

    /// A client in `region` signing with `credentials`, for long runs of
    /// requests such as a restic snapshot.
    ///
    /// Retries and connect timeout are explicit (`behavior_version_latest()`
    /// disables retries today): unretried, one connect failure fails the whole
    /// run. No attempt timeout, since a pack upload is unbounded.
    pub fn with_credentials(region: impl Into<String>, credentials: S3Credentials) -> Self {
        let config = credentials_config(region.into(), credentials.signing());
        Self::new(aws_sdk_s3::Client::from_conf(config), None)
    }

    /// A client on an S3-compatible endpoint such as `MinIO`, path-style.
    #[cfg(any(test, feature = "test-support"))]
    pub fn at_endpoint(endpoint: &str, region: &str, credentials: S3Credentials) -> Self {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .region(Region::new(region.to_string()))
            .credentials_provider(credentials.signing())
            .endpoint_url(endpoint)
            .force_path_style(true)
            .build();
        Self::new(aws_sdk_s3::Client::from_conf(config), None)
    }

    /// Create `bucket`, ignoring the "it is already there" answer.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn ensure_bucket(&self, bucket: &str) {
        _ = self.client.create_bucket().bucket(bucket).send().await;
    }

    pub(crate) fn new(
        client: aws_sdk_s3::Client,
        credentials_provider: Option<SharedCredentialsProvider>,
    ) -> Self {
        Self {
            client,
            credentials_provider,
        }
    }

    pub(super) fn credentials_provider(&self) -> Option<&SharedCredentialsProvider> {
        self.credentials_provider.as_ref()
    }
}

fn credentials_config(region: String, credentials: Credentials) -> aws_sdk_s3::Config {
    aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new(region))
        .credentials_provider(credentials)
        .retry_config(RetryConfig::standard().with_max_attempts(MAX_ATTEMPTS))
        .timeout_config(
            TimeoutConfig::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build(),
        )
        .build()
}

#[cfg(test)]
mod tests {
    use aws_sdk_s3::config::retry::RetryMode;

    use super::*;

    /// A connect that never completes is the fleet's dominant snapshot failure,
    /// and the SDK's own defaults move with its version, so the client states
    /// both itself.
    #[test]
    fn a_credentials_client_retries_a_request_that_cannot_connect() {
        let config = credentials_config(
            "eu-north-1".into(),
            Credentials::new("key", "secret", None, None, "test"),
        );
        assert_eq!(
            config.retry_config().map(RetryConfig::max_attempts),
            Some(MAX_ATTEMPTS)
        );
        // `adaptive()` would satisfy the count above and be a different client:
        // it adds a client-side rate limiter that throttles ahead of the request.
        assert_eq!(
            config.retry_config().map(RetryConfig::mode),
            Some(RetryMode::Standard)
        );
        assert_eq!(
            config
                .timeout_config()
                .and_then(TimeoutConfig::connect_timeout),
            Some(CONNECT_TIMEOUT)
        );
    }
}
