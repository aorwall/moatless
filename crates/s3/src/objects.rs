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

/// Delete every object under `prefix` in `bucket`. Returns the count deleted.
pub async fn delete_objects_by_prefix(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
) -> Result<usize, S3Error> {
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
            .map_err(|e| S3Error::ListObjects(Box::new(e)))?;

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
