use crate::{S3Error, S3Storage};
use aws_sdk_s3::config::{ProvideCredentials, SharedCredentialsProvider};
use aws_sdk_s3::presigning::PresigningConfig;
use std::time::{Duration, SystemTime};

const CREDENTIAL_EXPIRY_SKEW: Duration = Duration::from_secs(5);
const MIN_EFFECTIVE_PRESIGN_TTL: Duration = Duration::from_secs(1);

/// A presigned S3 URL and the lifetime callers should rely on.
pub struct PresignedUrl {
    pub url: String,
    pub expires_in: Duration,
}

/// Presign an HTTP `PUT` URL for `key`, valid for `ttl`.
pub async fn presign_put(
    storage: &S3Storage,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<PresignedUrl, S3Error> {
    let ttl = effective_presign_ttl(storage.credentials_provider(), ttl).await?;
    let cfg = PresigningConfig::expires_in(ttl).map_err(S3Error::InvalidPresignConfig)?;
    let req = storage
        .client
        .put_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| S3Error::PresignPut(Box::new(e)))?;
    Ok(PresignedUrl {
        url: req.uri().to_string(),
        expires_in: ttl,
    })
}

/// Presign an HTTP `GET` URL for `key`, valid for `ttl`.
pub async fn presign_get(
    storage: &S3Storage,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<PresignedUrl, S3Error> {
    let ttl = effective_presign_ttl(storage.credentials_provider(), ttl).await?;
    let cfg = PresigningConfig::expires_in(ttl).map_err(S3Error::InvalidPresignConfig)?;
    let req = storage
        .client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| S3Error::PresignGet(Box::new(e)))?;
    Ok(PresignedUrl {
        url: req.uri().to_string(),
        expires_in: ttl,
    })
}

/// Presign an HTTP `DELETE` URL for `key`, valid for `ttl`.
pub async fn presign_delete(
    storage: &S3Storage,
    bucket: &str,
    key: &str,
    ttl: Duration,
) -> Result<PresignedUrl, S3Error> {
    let ttl = effective_presign_ttl(storage.credentials_provider(), ttl).await?;
    let cfg = PresigningConfig::expires_in(ttl).map_err(S3Error::InvalidPresignConfig)?;
    let req = storage
        .client
        .delete_object()
        .bucket(bucket)
        .key(key)
        .presigned(cfg)
        .await
        .map_err(|e| S3Error::PresignDelete(Box::new(e)))?;
    Ok(PresignedUrl {
        url: req.uri().to_string(),
        expires_in: ttl,
    })
}

async fn effective_presign_ttl(
    credentials_provider: Option<&SharedCredentialsProvider>,
    requested_ttl: Duration,
) -> Result<Duration, S3Error> {
    let Some(credentials_provider) = credentials_provider else {
        return Ok(requested_ttl);
    };

    let credentials = credentials_provider
        .provide_credentials()
        .await
        .map_err(|e| S3Error::Credentials(Box::new(e)))?;
    let Some(expiry) = credentials.expiry() else {
        return Ok(requested_ttl);
    };

    let remaining = expiry
        .duration_since(SystemTime::now())
        .map_err(|_| S3Error::CredentialsExpired)?;
    let Some(effective) = remaining.checked_sub(CREDENTIAL_EXPIRY_SKEW) else {
        return Err(S3Error::CredentialsExpiring);
    };
    if effective < MIN_EFFECTIVE_PRESIGN_TTL {
        return Err(S3Error::CredentialsExpiring);
    }

    Ok(requested_ttl.min(effective))
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

        assert!(matches!(err, S3Error::CredentialsExpiring));
    }

    #[tokio::test]
    async fn effective_presign_ttl_rejects_expired_credentials() {
        let credentials = Credentials::new(
            "AKID",
            "SECRET",
            None,
            Some(SystemTime::now() - Duration::from_secs(1)),
            "test",
        );
        let provider = SharedCredentialsProvider::new(credentials);
        assert!(matches!(
            effective_presign_ttl(Some(&provider), Duration::from_mins(15)).await,
            Err(S3Error::CredentialsExpired)
        ));
    }

    #[tokio::test]
    async fn public_signing_operations_scope_urls_without_extra_headers() {
        let provider =
            SharedCredentialsProvider::new(Credentials::new("AKID", "SECRET", None, None, "test"));
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .credentials_provider(provider.clone())
            .endpoint_url("http://localhost:9000")
            .force_path_style(true)
            .build();
        let storage = S3Storage::new(aws_sdk_s3::Client::from_conf(config), Some(provider));
        let ttl = Duration::from_mins(1);
        let put = crate::presign_put(&storage, "bucket", "path/key", ttl)
            .await
            .unwrap();
        let get = crate::presign_get(&storage, "bucket", "path/key", ttl)
            .await
            .unwrap();
        let delete = crate::presign_delete(&storage, "bucket", "path/key", ttl)
            .await
            .unwrap();
        for signed in [&put, &get, &delete] {
            assert_eq!(signed.expires_in, ttl);
            assert!(
                signed
                    .url
                    .starts_with("http://localhost:9000/bucket/path/key?")
            );
            assert!(signed.url.contains("X-Amz-SignedHeaders=host"));
            assert!(signed.url.contains("X-Amz-Expires=60"));
        }
        assert_ne!(put.url, get.url);
        assert_ne!(get.url, delete.url);
        assert_ne!(put.url, delete.url);
    }
}
