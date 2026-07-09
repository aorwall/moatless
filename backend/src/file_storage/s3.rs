//! S3 operations for file storage.

use std::time::Duration;

use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;

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
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<String, String> {
    let cfg =
        PresigningConfig::expires_in(ttl).map_err(|e| format!("presign config invalid: {e}"))?;
    let req = client
        .put_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| format!("S3 presign put_object failed: {e}"))?;
    Ok(req.uri().to_string())
}

/// Presign an HTTP `GET` URL for `key`, valid for `ttl`.
///
/// Scoped to this bucket + key + `GET` verb. A `GET` against a missing object
/// returns HTTP 404, which the sandbox treats as "nothing stored yet".
pub async fn presign_get(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<String, String> {
    let cfg =
        PresigningConfig::expires_in(ttl).map_err(|e| format!("presign config invalid: {e}"))?;
    let req = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| format!("S3 presign get_object failed: {e}"))?;
    Ok(req.uri().to_string())
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
