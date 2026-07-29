//! S3 operations for file storage.

use std::time::{Duration, SystemTime};

use aws_sdk_s3::config::{ProvideCredentials, SharedCredentialsProvider};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;

const CREDENTIAL_EXPIRY_SKEW: Duration = Duration::from_secs(5);
const MIN_EFFECTIVE_PRESIGN_TTL: Duration = Duration::from_secs(1);

/// Shared S3 handle plus the credentials provider used to build it.
///
/// The generated S3 client does not expose its credentials provider, but
/// presigned URLs are only usable while the signing credentials remain valid.
/// Keeping the provider beside the client lets callers report the effective URL
/// lifetime instead of always promising the requested TTL.
#[derive(Clone)]
pub struct S3Storage {
    pub client: aws_sdk_s3::Client,
    credentials_provider: Option<SharedCredentialsProvider>,
}

impl S3Storage {
    pub fn new(
        client: aws_sdk_s3::Client,
        credentials_provider: Option<SharedCredentialsProvider>,
    ) -> Self {
        Self {
            client,
            credentials_provider,
        }
    }

    fn credentials_provider(&self) -> Option<&SharedCredentialsProvider> {
        self.credentials_provider.as_ref()
    }
}

/// A presigned S3 URL and the lifetime callers should rely on.
pub struct PresignedUrl {
    pub url: String,
    pub expires_in: Duration,
}

/// Upload a file to S3.
pub async fn put_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    body: Vec<u8>,
    content_type: &str,
) -> Result<(), String> {
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(body))
        .content_type(content_type)
        .send()
        .await
        .map_err(|e| format!("S3 put_object failed: {e}"))?;

    Ok(())
}

/// Download a file from S3.
pub async fn get_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<(Vec<u8>, String), String> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| format!("S3 get_object failed: {e}"))?;

    let content_type = resp
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = resp
        .body
        .collect()
        .await
        .map_err(|e| format!("Failed to read S3 body: {e}"))?
        .into_bytes()
        .to_vec();

    Ok((bytes, content_type))
}

/// Download a file from S3, returning `Ok(None)` when the key does not exist.
///
/// Distinguishes a missing object (expected — e.g. a task with no transcript yet)
/// from a real error, so callers can treat "not found" as an empty result instead
/// of logging a spurious failure.
pub async fn get_object_opt(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<Option<Vec<u8>>, String> {
    match client.get_object().bucket(bucket).key(key).send().await {
        Ok(resp) => {
            let bytes = resp
                .body
                .collect()
                .await
                .map_err(|e| format!("Failed to read S3 body: {e}"))?
                .into_bytes()
                .to_vec();
            Ok(Some(bytes))
        }
        Err(e) => match e.as_service_error() {
            Some(se) if se.is_no_such_key() => Ok(None),
            _ => Err(format!("S3 get_object failed: {e}")),
        },
    }
}

/// Presign an HTTP `PUT` URL for `key`, valid for `ttl`.
///
/// The URL is scoped to exactly this bucket + key + `PUT` verb. A holder can
/// upload that one object until the URL expires and nothing else — this is how
/// the `s3` storage backend lets an AWS-credential-less sandbox write its
/// workspace archive directly to S3 without a standing IAM identity. No headers
/// are bound into the signature, so the caller can `PUT` the body with a plain
/// HTTP client and no special headers.
pub async fn presign_put(
    storage: &S3Storage,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<PresignedUrl, String> {
    let ttl = effective_presign_ttl(storage.credentials_provider(), ttl).await?;
    let cfg =
        PresigningConfig::expires_in(ttl).map_err(|e| format!("presign config invalid: {e}"))?;
    let req = storage
        .client
        .put_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| format!("S3 presign put_object failed: {e}"))?;
    Ok(PresignedUrl {
        url: req.uri().to_string(),
        expires_in: ttl,
    })
}

/// Presign an HTTP `GET` URL for `key`, valid for `ttl`.
///
/// Scoped to this bucket + key + `GET` verb. A `GET` against a missing object
/// returns HTTP 404, which the sandbox treats as "nothing stored yet".
pub async fn presign_get(
    storage: &S3Storage,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<PresignedUrl, String> {
    let ttl = effective_presign_ttl(storage.credentials_provider(), ttl).await?;
    let cfg =
        PresigningConfig::expires_in(ttl).map_err(|e| format!("presign config invalid: {e}"))?;
    let req = storage
        .client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| format!("S3 presign get_object failed: {e}"))?;
    Ok(PresignedUrl {
        url: req.uri().to_string(),
        expires_in: ttl,
    })
}

async fn effective_presign_ttl(
    credentials_provider: Option<&SharedCredentialsProvider>,
    requested_ttl: Duration,
) -> Result<Duration, String> {
    let Some(credentials_provider) = credentials_provider else {
        return Ok(requested_ttl);
    };

    let credentials = credentials_provider
        .provide_credentials()
        .await
        .map_err(|e| format!("failed to load AWS credentials for presign TTL: {e}"))?;
    let Some(expiry) = credentials.expiry() else {
        return Ok(requested_ttl);
    };

    let remaining = expiry
        .duration_since(SystemTime::now())
        .map_err(|_| "AWS credentials expired before S3 presign".to_string())?;
    let Some(effective) = remaining.checked_sub(CREDENTIAL_EXPIRY_SKEW) else {
        return Err("AWS credentials expire too soon for S3 presign".to_string());
    };
    if effective < MIN_EFFECTIVE_PRESIGN_TTL {
        return Err("AWS credentials expire too soon for S3 presign".to_string());
    }

    Ok(requested_ttl.min(effective))
}

/// Delete every object under `prefix` in `bucket`. Returns the count deleted.
///
/// Paginates `list_objects_v2` and deletes each key individually (a task's
/// transcript prefix holds a single object today, so batching buys nothing).
pub async fn delete_objects_by_prefix(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
) -> Result<usize, String> {
    let mut deleted = 0usize;
    let mut continuation: Option<String> = None;

    loop {
        let mut req = client.list_objects_v2().bucket(bucket).prefix(prefix);
        if let Some(token) = &continuation {
            req = req.continuation_token(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("S3 list_objects_v2 failed: {e}"))?;

        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                client
                    .delete_object()
                    .bucket(bucket)
                    .key(key)
                    .send()
                    .await
                    .map_err(|e| format!("S3 delete_object failed: {e}"))?;
                deleted += 1;
            }
        }

        match resp.next_continuation_token() {
            Some(token) if resp.is_truncated().unwrap_or(false) => {
                continuation = Some(token.to_string());
            }
            _ => break,
        }
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_s3::config::Credentials;

    #[tokio::test]
    async fn effective_presign_ttl_uses_requested_ttl_without_expiring_credentials() {
        let ttl = effective_presign_ttl(None, Duration::from_secs(900))
            .await
            .unwrap();

        assert_eq!(ttl, Duration::from_secs(900));
    }

    #[tokio::test]
    async fn effective_presign_ttl_is_capped_by_credential_expiry() {
        let credentials = Credentials::new(
            "AKID",
            "SECRET",
            None,
            Some(SystemTime::now() + Duration::from_secs(120)),
            "test",
        );
        let provider = SharedCredentialsProvider::new(credentials);

        let ttl = effective_presign_ttl(Some(&provider), Duration::from_secs(900))
            .await
            .unwrap();

        assert!(ttl <= Duration::from_secs(115));
        assert!(ttl > Duration::from_secs(110));
    }

    #[tokio::test]
    async fn effective_presign_ttl_rejects_credentials_expiring_inside_skew() {
        let credentials = Credentials::new(
            "AKID",
            "SECRET",
            None,
            Some(SystemTime::now() + Duration::from_secs(4)),
            "test",
        );
        let provider = SharedCredentialsProvider::new(credentials);

        let err = effective_presign_ttl(Some(&provider), Duration::from_secs(900))
            .await
            .unwrap_err();

        assert_eq!(err, "AWS credentials expire too soon for S3 presign");
    }
}
