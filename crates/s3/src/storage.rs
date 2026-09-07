use aws_sdk_s3::config::SharedCredentialsProvider;

/// Shared S3 handle plus the credentials provider used to build it.
#[derive(Clone)]
pub struct S3Storage {
    pub client: aws_sdk_s3::Client,
    credentials_provider: Option<SharedCredentialsProvider>,
}

impl S3Storage {
    /// Pair the client with the credentials provider used to construct it.
    pub fn new(
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
