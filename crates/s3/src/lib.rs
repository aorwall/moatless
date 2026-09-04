// Allow unwrap/expect in test code — production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! S3 object operations and presigning.
//!
//! A leaf over `aws-sdk-s3` with no in-workspace dependency, so the SDK
//! compiles in parallel with the band graph instead of gating it.

use std::time::{Duration, SystemTime};

use aws_sdk_s3::config::{ProvideCredentials, SharedCredentialsProvider};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};

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

/// Presign an HTTP `DELETE` URL for `key`, valid for `ttl`.
///
/// Scoped to this bucket + key + `DELETE` verb, so the holder can remove that
/// one object and nothing else. The workspace file mirror uses it to drop the
/// objects for files the agent deleted, without giving the sandbox an S3
/// identity that could delete anything else.
pub async fn presign_delete(
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
        .delete_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| format!("S3 presign delete_object failed: {e}"))?;
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

/// When `key` was last written, or `Ok(None)` when it does not exist.
///
/// The three answers are deliberately distinct. `Ok(Some(t))` proves the object
/// is there and how old it is; `Ok(None)` proves it is not there; `Err` means
/// the question could not be answered. A caller deciding whether a snapshot is
/// safe to rely on must not read the third as the second — "cannot tell" is not
/// "not there".
///
/// `HEAD` rather than `GET`: the workspace archive can be hundreds of MB and
/// only its timestamp is wanted.
pub async fn head_object_modified_at(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    match client.head_object().bucket(bucket).key(key).send().await {
        Ok(resp) => {
            let Some(modified) = resp.last_modified() else {
                return Err(format!("S3 head_object returned no LastModified for {key}"));
            };
            chrono::DateTime::from_timestamp(modified.secs(), modified.subsec_nanos())
                .map(Some)
                .ok_or_else(|| format!("S3 LastModified out of range for {key}"))
        }
        Err(e) => match e.as_service_error() {
            Some(se) if se.is_not_found() => Ok(None),
            _ => Err(format!("S3 head_object failed: {e}")),
        },
    }
}

/// Delete one object. Deleting a key that is not there succeeds, which is what
/// S3 itself reports and what a caller undoing a half-finished upload wants.
pub async fn delete_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<(), String> {
    client
        .delete_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| format!("S3 delete_object failed: {e}"))?;

    Ok(())
}

/// Delete every object under `prefix` in `bucket`. Returns the count deleted.
///
/// Paginates `list_objects_v2` and deletes each page with one `delete_objects`
/// call. The page and the batch are both 1,000 keys, so a task's prefix costs
/// two requests per thousand objects however many it holds — a workspace file
/// mirror puts thousands under one task, where a per-key delete would be
/// thousands of round trips.
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

        let keys: Vec<String> = resp
            .contents()
            .iter()
            .filter_map(|obj| obj.key().map(str::to_string))
            .collect();
        deleted += delete_keys(client, bucket, &keys).await?;

        match resp.next_continuation_token() {
            Some(token) if resp.is_truncated().unwrap_or(false) => {
                continuation = Some(token.to_string());
            }
            _ => break,
        }
    }

    Ok(deleted)
}

/// Delete up to 1,000 keys in one call. Returns the count deleted.
///
/// `delete_objects` reports per-key failures in the response body rather than as
/// a request error, so a caller that only checked the request would count a
/// silent partial delete as a full one.
async fn delete_keys(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    keys: &[String],
) -> Result<usize, String> {
    if keys.is_empty() {
        return Ok(0);
    }

    let mut objects = Vec::with_capacity(keys.len());
    for key in keys {
        objects.push(
            ObjectIdentifier::builder()
                .key(key)
                .build()
                .map_err(|e| format!("S3 delete identifier invalid: {e}"))?,
        );
    }
    let delete = Delete::builder()
        .set_objects(Some(objects))
        .quiet(true)
        .build()
        .map_err(|e| format!("S3 delete request invalid: {e}"))?;

    let resp = client
        .delete_objects()
        .bucket(bucket)
        .delete(delete)
        .send()
        .await
        .map_err(|e| format!("S3 delete_objects failed: {e}"))?;

    let failures = resp.errors();
    if !failures.is_empty() {
        let first = failures
            .first()
            .map(|e| format!("{}: {}", e.key().unwrap_or("?"), e.message().unwrap_or("?")))
            .unwrap_or_default();
        return Err(format!(
            "S3 delete_objects left {} of {} keys: {first}",
            failures.len(),
            keys.len()
        ));
    }

    Ok(keys.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_s3::config::Credentials;

    #[tokio::test]
    async fn effective_presign_ttl_uses_requested_ttl_without_expiring_credentials() {
        let ttl = effective_presign_ttl(None, Duration::from_mins(15))
            .await
            .unwrap();

        assert_eq!(ttl, Duration::from_mins(15));
    }

    #[tokio::test]
    async fn effective_presign_ttl_is_capped_by_credential_expiry() {
        let credentials = Credentials::new(
            "AKID",
            "SECRET",
            None,
            Some(SystemTime::now() + Duration::from_mins(2)),
            "test",
        );
        let provider = SharedCredentialsProvider::new(credentials);

        let ttl = effective_presign_ttl(Some(&provider), Duration::from_mins(15))
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

        let err = effective_presign_ttl(Some(&provider), Duration::from_mins(15))
            .await
            .unwrap_err();

        assert_eq!(err, "AWS credentials expire too soon for S3 presign");
    }
}
