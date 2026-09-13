use crate::S3Error;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};

/// Upload a file to S3.
pub async fn put_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    body: Vec<u8>,
    content_type: &str,
) -> Result<(), S3Error> {
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(body))
        .content_type(content_type)
        .send()
        .await
        .map_err(|e| S3Error::PutObject(Box::new(e)))?;

    Ok(())
}

/// Download a file from S3.
pub async fn get_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<(Vec<u8>, String), S3Error> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| S3Error::GetObject(Box::new(e)))?;

    let content_type = resp
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = resp
        .body
        .collect()
        .await
        .map_err(S3Error::ReadBody)?
        .into_bytes()
        .to_vec();

    Ok((bytes, content_type))
}

/// Download a file from S3, returning `Ok(None)` when the key does not exist.
pub async fn get_object_opt(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<Option<Vec<u8>>, S3Error> {
    match client.get_object().bucket(bucket).key(key).send().await {
        Ok(resp) => {
            let bytes = resp
                .body
                .collect()
                .await
                .map_err(S3Error::ReadBody)?
                .into_bytes()
                .to_vec();
            Ok(Some(bytes))
        }
        Err(e) => match e.as_service_error() {
            Some(se) if se.is_no_such_key() => Ok(None),
            _ => Err(S3Error::GetObject(Box::new(e))),
        },
    }
}

/// When `key` was last written, or `Ok(None)` when it does not exist.
pub async fn head_object_modified_at(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, S3Error> {
    match client.head_object().bucket(bucket).key(key).send().await {
        Ok(resp) => {
            let Some(modified) = resp.last_modified() else {
                return Err(S3Error::MissingLastModified {
                    key: key.to_string(),
                });
            };
            chrono::DateTime::from_timestamp(modified.secs(), modified.subsec_nanos())
                .map(Some)
                .ok_or_else(|| S3Error::InvalidLastModified {
                    key: key.to_string(),
                })
        }
        Err(e) => match e.as_service_error() {
            Some(se) if se.is_not_found() => Ok(None),
            _ => Err(S3Error::HeadObject(Box::new(e))),
        },
    }
}

/// Delete one object. Deleting a key that is not there succeeds, which is what
pub async fn delete_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<(), S3Error> {
    client
        .delete_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| S3Error::DeleteObject(Box::new(e)))?;

    Ok(())
}

/// Walk every page of a prefix listing, handing each page's objects to `page`.
///
/// Keys on `NextContinuationToken` alone: S3 sets it only when truncated, so
/// trusting anything else would silently stop the walk early.
async fn for_each_page<F, Fut>(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
    mut page: F,
) -> Result<usize, S3Error>
where
    F: FnMut(Vec<(String, u64)>) -> Fut,
    Fut: std::future::Future<Output = Result<usize, S3Error>>,
{
    let mut total = 0usize;
    let mut continuation: Option<String> = None;

    loop {
        let mut req = client.list_objects_v2().bucket(bucket).prefix(prefix);
        if let Some(token) = &continuation {
            req = req.continuation_token(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| S3Error::ListObjects(Box::new(e)))?;

        let objects: Vec<(String, u64)> = resp
            .contents()
            .iter()
            .filter_map(|obj| {
                let key = obj.key()?;
                Some((key.to_string(), obj.size().unwrap_or(0).max(0) as u64))
            })
            .collect();
        total += page(objects).await?;

        match resp.next_continuation_token() {
            // Not `is_truncated`: S3 sets the token only on a truncated
            // response, and a store that omits the flag would stop the walk a
            // page early. The token is therefore the only bound this loop has,
            // so one that does not advance ends it rather than spinning.
            Some(token) if continuation.as_deref() != Some(token) => {
                continuation = Some(token.to_string());
            }
            _ => break,
        }
    }

    Ok(total)
}

/// Delete every object under `prefix` in `bucket`. Returns the count deleted.
pub async fn delete_objects_by_prefix(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
) -> Result<usize, S3Error> {
    for_each_page(client, bucket, prefix, |objects| {
        let keys: Vec<String> = objects.into_iter().map(|(key, _)| key).collect();
        async move { delete_keys(client, bucket, &keys).await }
    })
    .await
}

/// Delete up to 1,000 keys in one call. Returns the count deleted.
async fn delete_keys(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    keys: &[String],
) -> Result<usize, S3Error> {
    if keys.is_empty() {
        return Ok(0);
    }

    let mut objects = Vec::with_capacity(keys.len());
    for key in keys {
        objects.push(
            ObjectIdentifier::builder()
                .key(key)
                .build()
                .map_err(S3Error::InvalidDeleteIdentifier)?,
        );
    }
    let delete = Delete::builder()
        .set_objects(Some(objects))
        .quiet(true)
        .build()
        .map_err(S3Error::InvalidDeleteRequest)?;

    let resp = client
        .delete_objects()
        .bucket(bucket)
        .delete(delete)
        .send()
        .await
        .map_err(|e| S3Error::DeleteObjects(Box::new(e)))?;

    let failures = resp.errors();
    if !failures.is_empty() {
        return Err(S3Error::BatchDelete {
            attempted: keys.len(),
            failures: failures.to_vec(),
        });
    }

    Ok(keys.len())
}

/// Download `length` bytes of `key` starting at `offset`.
///
/// The range is inclusive on the wire (`bytes=<first>-<last>`); a zero `length`
/// has no valid encoding and is rejected instead of becoming a whole-object
/// GET.
pub async fn get_object_range(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    offset: u64,
    length: u64,
) -> Result<Vec<u8>, S3Error> {
    if length == 0 {
        return Err(S3Error::EmptyRange {
            key: key.to_string(),
        });
    }
    let last = offset + length - 1;

    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .range(format!("bytes={offset}-{last}"))
        .send()
        .await
        .map_err(|e| S3Error::GetObject(Box::new(e)))?;

    Ok(resp
        .body
        .collect()
        .await
        .map_err(S3Error::ReadBody)?
        .into_bytes()
        .to_vec())
}

/// The size of `key` in bytes, or `Ok(None)` when it does not exist.
///
/// Existence comes from the response status alone, needing no `Last-Modified`
/// header unlike [`head_object_modified_at`].
pub async fn head_object_size(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<Option<u64>, S3Error> {
    match client.head_object().bucket(bucket).key(key).send().await {
        Ok(resp) => Ok(Some(resp.content_length().unwrap_or(0).max(0) as u64)),
        Err(e) => match e.as_service_error() {
            Some(se) if se.is_not_found() => Ok(None),
            _ => Err(S3Error::HeadObject(Box::new(e))),
        },
    }
}

/// Whether `bucket` holds any object under `prefix`.
///
/// Asks for one key, so cost does not grow with the prefix; use
/// [`head_object_modified_at`] instead when the key is already known.
pub async fn prefix_has_any_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
) -> Result<bool, S3Error> {
    let resp = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(prefix)
        .max_keys(1)
        .send()
        .await
        .map_err(|e| S3Error::ListObjects(Box::new(e)))?;

    Ok(!resp.contents().is_empty())
}

/// Every key under `prefix` in `bucket`, with its size in bytes.
pub async fn list_objects_with_size(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<(String, u64)>, S3Error> {
    let mut found = Vec::new();

    for_each_page(client, bucket, prefix, |objects| {
        found.extend(objects);
        async { Ok(0) }
    })
    .await?;

    Ok(found)
}
