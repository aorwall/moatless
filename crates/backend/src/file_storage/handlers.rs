//! File storage HTTP handlers.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use axum::Json;
use axum_extra::extract::Multipart;

use std::sync::Arc;

use moatless_identity::extractors::AuthUser;
use moatless_types::access_control::ScopedResource;
use moatless_core::config::S3Config;

use super::dao::FileDao;
use super::error::FileStorageError;
use super::s3;
use super::schemas::UploadResponse;
use super::store;

/// Upload a file via multipart form data.
#[utoipa::path(
    post,
    path = "/api/v1/uploads",
    tag = "file-storage",
    request_body(content_type = "multipart/form-data", content = super::schemas::UploadFileBody),
    responses(
        (status = 200, description = "File uploaded successfully", body = UploadResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn upload_file(
    State(s3_storage): State<Option<s3::S3Storage>>,
    State(s3_config): State<Arc<S3Config>>,
    State(files): State<Arc<dyn FileDao>>,
    auth: AuthUser,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, FileStorageError> {
    // Checked before the body is read as well as inside the store: a
    // deployment without S3 refuses every upload, and there is no reason to
    // buffer megabytes first to say so.
    if s3_storage.is_none() || s3_config.bucket_name.is_none() {
        return Err(FileStorageError::S3NotConfigured);
    }

    let mut file_data = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| FileStorageError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let filename = field.file_name().unwrap_or("unnamed").to_string();
            let content_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            let data = field
                .bytes()
                .await
                .map_err(|e| FileStorageError::UploadFailed(format!("Read error: {e}")))?;

            file_data = Some((filename, content_type, data.to_vec()));
        }
    }

    let (filename, content_type, data) =
        file_data.ok_or_else(|| FileStorageError::BadRequest("No file field found".to_string()))?;

    let meta = store::store_upload(
        s3_storage.as_ref(),
        &s3_config,
        files.as_ref(),
        &auth.scope_context(),
        store::NewUpload {
            filename,
            mime_type: content_type,
            data,
            file_id: None,
        },
    )
    .await?;

    Ok(Json(UploadResponse {
        file_id: meta.file_id,
        filename: meta.filename,
        mime_type: meta.mime_type,
        size: meta.size,
    }))
}

/// Download a file by ID.
#[utoipa::path(
    get,
    path = "/api/v1/uploads/{file_id}",
    tag = "file-storage",
    params(
        ("file_id" = String, Path, description = "File identifier")
    ),
    responses(
        (status = 200, description = "File content", content_type = "application/octet-stream"),
        (status = 404, description = "File not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn download_file(
    State(s3_storage): State<Option<s3::S3Storage>>,
    State(s3_config): State<Arc<S3Config>>,
    State(files): State<Arc<dyn FileDao>>,
    auth: AuthUser,
    Path(file_id): Path<String>,
) -> Result<Response, FileStorageError> {
    let s3_client = s3_storage
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let bucket = s3_config
        .bucket_name
        .as_ref()
        .ok_or(FileStorageError::S3NotConfigured)?;

    let meta = files
        .find_by_id(&file_id)
        .await?
        .ok_or_else(|| FileStorageError::NotFound(file_id.clone()))?;
    let resource = ScopedResource::borrowed(
        meta.scope,
        meta.owner_user_id.as_deref().or(meta.created_by.as_deref()),
    );
    if !auth.scope_context().can_read(&resource) {
        return Err(FileStorageError::Forbidden(format!(
            "No access to file {file_id}"
        )));
    }

    let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
    let (data, content_type) = s3::get_object(&s3_client.client, bucket, &s3_key)
        .await
        .map_err(FileStorageError::UploadFailed)?;

    #[allow(clippy::expect_used)]
    // Response::builder() only fails with invalid header names (ours are static)
    let response = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(&meta.filename),
        )
        .body(Body::from(data))
        .expect("valid response with static header names");

    Ok(response)
}

/// `Content-Disposition` for a download, as RFC 6266 wants it.
///
/// A filename is whatever the person called their file, and header values are
/// bytes: a quote or a backslash in one ends the quoted string early, and a
/// newline splits the response. So the `filename` parameter carries a sanitized
/// ASCII fallback — every byte outside a conservative allowlist replaced with
/// `_` — and the real name is carried beside it in the RFC 5987 `filename*`
/// form, which every current browser prefers and which is percent-encoded and
/// therefore cannot carry a delimiter at all.
fn content_disposition(filename: &str) -> String {
    use std::fmt::Write;

    let mut fallback = String::with_capacity(filename.len());
    let mut encoded = String::with_capacity(filename.len());
    for byte in filename.bytes() {
        match byte {
            // `attr-char` from RFC 5987, which is what `filename*` may carry
            // unescaped. Everything else — including the space and every
            // non-ASCII byte — is percent-encoded.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'!' | b'#' | b'$' | b'&' | b'+' | b'-'
            | b'.' | b'^' | b'_' | b'`' | b'|' | b'~' => {
                encoded.push(byte as char);
                fallback.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
                // The fallback keeps a space and a few punctuation marks a name
                // commonly has, and loses everything else to `_`.
                fallback.push(match byte {
                    b' ' | b'(' | b')' | b',' | b'=' | b'@' | b'[' | b']' | b'{' | b'}' => {
                        byte as char
                    }
                    _ => '_',
                });
            }
        }
    }
    if fallback.is_empty() {
        fallback.push_str("download");
    }

    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::content_disposition;

    /// A name that is already plain survives unchanged in both forms.
    #[test]
    fn an_ordinary_name_reaches_both_parameters_intact() {
        assert_eq!(
            content_disposition("report.pdf"),
            "attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf"
        );
    }

    /// The bytes that would end the quoted string, or the header, or the
    /// response. None of them may reach either parameter.
    #[test]
    fn a_name_cannot_carry_a_delimiter_into_the_header() {
        let header = content_disposition("a\"; filename=\"evil.exe\r\nX-Evil: 1");

        assert!(!header.contains('"') || header.matches('"').count() == 2, "{header}");
        assert!(!header.contains('\r') && !header.contains('\n'), "{header}");
        assert!(!header.contains("evil.exe\r"), "{header}");
    }

    /// A name nobody wrote in ASCII still arrives whole, in the parameter that
    /// can carry it.
    #[test]
    fn a_non_ascii_name_survives_percent_encoded() {
        let header = content_disposition("rapport-å.txt");

        assert!(header.contains("filename*=UTF-8''rapport-%C3%A5.txt"), "{header}");
        assert!(header.contains("filename=\"rapport-__.txt\""), "{header}");
    }

    /// A name with no ASCII in it at all still gets a fallback, because a
    /// browser that cannot read `filename*` has to save the file as something.
    #[test]
    fn a_name_with_no_ascii_left_still_has_a_fallback() {
        let header = content_disposition("日本語");

        assert!(header.contains("filename=\"_________\""), "{header}");
        assert!(
            header.contains("filename*=UTF-8''%E6%97%A5%E6%9C%AC%E8%AA%9E"),
            "{header}"
        );
    }

    /// A name that is nothing at all — the multipart route stores `unnamed`
    /// rather than an empty string, but a row from anywhere else could.
    #[test]
    fn an_empty_name_is_replaced_rather_than_omitted() {
        assert_eq!(
            content_disposition(""),
            "attachment; filename=\"download\"; filename*=UTF-8''"
        );
    }
}
