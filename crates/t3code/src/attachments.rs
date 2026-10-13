//! Chat attachment uploads: minting a URL, taking its bytes, and removing one
//! that was abandoned.
//!
//! The credential is in the URL — a short-lived JWT naming the viewer and the
//! one attachment it may write — not a cookie, since a raw `xhr.send(file)`
//! sends no CSRF token; that also lets an API-key client upload.
//!
//! Name, mime type and size are validated and signed into the token at mint
//! time, so the route trusts the token, never the request's own headers.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures::StreamExt;
use moatless_types::access_control::ScopeContext;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, error, warn};

use crate::UiRpcState;
use crate::ports::{AttachmentStore, AttachmentStoreError, DeclaredAttachment};

/// Where [`create_upload_url`]'s answers point, and what [`receive`] is mounted
/// at; the rest of the path is the signed token.
///
/// Exempt from the auth middleware's cookie handling in
/// `moatless_identity::auth::middleware`, which knows this prefix by name.
pub(crate) const UPLOAD_ROUTE_PREFIX: &str = "/api/attachments";
pub(crate) const UPLOAD_ROUTE_PATTERN: &str = "/api/attachments/{token}";

/// `PROVIDER_SEND_TURN_MAX_IMAGE_BYTES` (`contracts/src/orchestration.ts:158`).
pub(crate) const MAX_IMAGE_BYTES: i64 = 10 * 1024 * 1024;

/// `PROVIDER_SEND_TURN_MAX_FILE_BYTES` (`contracts/src/orchestration.ts:159`),
/// and the ceiling this deployment reports as
/// `capabilities.fileAttachments.maxUploadBytes`.
pub(crate) const MAX_FILE_BYTES: i64 = 50 * 1024 * 1024;

/// `PROVIDER_SEND_TURN_SUPPORTED_IMAGE_MIME_TYPES`
/// (`contracts/src/orchestration.ts:160-165`). An `image` upload is restricted
/// to these; a `file` upload takes any type.
pub(crate) const SUPPORTED_IMAGE_MIME_TYPES: [&str; 4] =
    ["image/gif", "image/jpeg", "image/png", "image/webp"];

/// `TrimmedNonEmptyString.check(isMaxLength(255))` on the attachment's name.
const NAME_MAX_CHARS: usize = 255;

/// `TrimmedNonEmptyString.check(isMaxLength(100))` on its mime type.
const MIME_MAX_CHARS: usize = 100;

/// `ATTACHMENT_UPLOAD_URL_TTL_MS` (`contracts/src/assets.ts:50`).
///
/// Unlike the interval [`crate::assets`] reports, this one is enforced as the
/// token's `exp`, so the URL really does stop working.
const UPLOAD_TTL_SECS: i64 = 10 * 60;

const UPLOAD_TOKEN_ISSUER: &str = "moatless-vibe-backend/attachment-upload";

/// How much of a declared size is allocated before any of it arrives.
///
/// A declaration is only a claim — a client could mint a 50 MiB upload and send
/// nothing — so the buffer grows into the rest as bytes actually arrive.
const UPLOAD_PREALLOCATE_MAX: i64 = 1024 * 1024;

/// What a minted URL authorizes: one viewer, one attachment, one name, one
/// size, for ten minutes.
#[derive(Debug, Serialize, Deserialize)]
struct UploadClaims {
    /// The viewer who minted it. The stored upload is owned by them, and the
    /// route has no other identity to go on.
    sub: String,
    /// Whether that viewer is an admin, so the [`ScopeContext`] the store sees
    /// is the one the socket would have built.
    adm: bool,
    /// The attachment id, which is also the id the upload is stored under and
    /// the one `assets.createUrl` resolves to a download.
    aid: String,
    name: String,
    mime: String,
    size: i64,
    exp: u64,
    iss: String,
}

/// Why a URL could not be minted.
pub(crate) enum MintError {
    /// The payload is not one of the union's members. The client's own schema
    /// refuses these before they are sent, so this is a malformed request
    /// rather than one of the method's declared outcomes.
    Invalid(String),
    /// Nothing here can be signed, so no URL can be handed out. The contract
    /// names this failure, and the client renders it.
    NoSigningKey,
}

/// One attachment upload, as the contract's `AttachmentCreateUploadUrlInput`
/// union describes it.
struct Declared {
    name: String,
    mime_type: String,
    size_bytes: i64,
}

/// Answer `attachments.createUploadUrl`.
///
/// The id is a fresh uuid stored as the upload id, so an attachment id and a
/// Moatless upload id are the same string — which is what keeps the download,
/// the prompt block and the Sandbox pre-fetch working unchanged.
pub(crate) fn create_upload_url(
    identity: &UiRpcAuthIdentity,
    signing_key: Option<&str>,
    payload: &Value,
) -> Result<Value, MintError> {
    let declared = declared(payload)?;
    let signing_key = signing_key.ok_or(MintError::NoSigningKey)?;

    let attachment_id = uuid::Uuid::new_v4().to_string();
    let expires_at = Utc::now() + chrono::Duration::seconds(UPLOAD_TTL_SECS);
    let claims = UploadClaims {
        sub: identity.user_id.to_string(),
        adm: matches!(identity.role, moatless_types::ui_rpc::UserRole::Admin),
        aid: attachment_id.clone(),
        name: declared.name,
        mime: declared.mime_type,
        size: declared.size_bytes,
        #[allow(clippy::cast_sign_loss)]
        // `expires_at` is now plus ten minutes; it is not before the epoch.
        exp: expires_at.timestamp().max(0) as u64,
        iss: UPLOAD_TOKEN_ISSUER.to_string(),
    };

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(signing_key.as_bytes()),
    )
    .map_err(|error| {
        error!(%error, "Failed to sign a T3 attachment upload URL");
        MintError::NoSigningKey
    })?;

    Ok(json!({
        "attachmentId": attachment_id,
        "relativeUrl": format!("{UPLOAD_ROUTE_PREFIX}/{token}"),
        "expiresAt": expires_at.timestamp_millis()
    }))
}

/// The payload read against `AttachmentCreateUploadUrlInput`.
///
/// An `image` is restricted to four mime types and 10 MiB, a `file` takes any
/// type up to 50 MiB. `type` is optional on the image member, so a payload
/// without one is an image, as a pre-files client sends.
fn declared(payload: &Value) -> Result<Declared, MintError> {
    let kind = match payload.get("type") {
        None | Some(Value::Null) => "image",
        Some(Value::String(kind)) => kind.as_str(),
        Some(_) => return Err(MintError::Invalid("`type` must be a string".to_string())),
    };

    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.chars().count() <= NAME_MAX_CHARS)
        .ok_or_else(|| MintError::Invalid("`name` must be a non-empty file name".to_string()))?
        .to_string();

    let mime_type = payload
        .get("mimeType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mime| !mime.is_empty() && mime.chars().count() <= MIME_MAX_CHARS)
        .ok_or_else(|| MintError::Invalid("`mimeType` must be a mime type".to_string()))?
        .to_ascii_lowercase();

    let size_bytes = payload
        .get("sizeBytes")
        .and_then(Value::as_i64)
        .ok_or_else(|| MintError::Invalid("`sizeBytes` must be a number".to_string()))?;

    let max_bytes = match kind {
        "image" => {
            if !SUPPORTED_IMAGE_MIME_TYPES.contains(&mime_type.as_str()) {
                return Err(MintError::Invalid(format!(
                    "`{mime_type}` is not an image type this server accepts"
                )));
            }
            MAX_IMAGE_BYTES
        }
        "file" => MAX_FILE_BYTES,
        other => {
            return Err(MintError::Invalid(format!(
                "`{other}` is not an attachment type"
            )));
        }
    };
    if !(1..=max_bytes).contains(&size_bytes) {
        return Err(MintError::Invalid(format!(
            "`sizeBytes` must be between 1 and {max_bytes}"
        )));
    }

    Ok(Declared {
        name,
        mime_type,
        size_bytes,
    })
}

/// Answer `attachments.delete`.
///
/// Deleting an attachment never uploaded, or already deleted, succeeds: the
/// composer calls this on any abandoned file (cancel, retry, discarded draft)
/// with no way to know which state the server is in.
pub(crate) async fn delete(
    store: &dyn AttachmentStore,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), String> {
    let attachment_id = payload
        .get("attachmentId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| is_attachment_id(id))
        .ok_or_else(|| "attachments.delete requires an attachmentId".to_string())?;

    match store.delete(viewer, attachment_id).await {
        Ok(()) => Ok(()),
        Err(AttachmentStoreError::Forbidden) => {
            // Somebody else's attachment is, from here, one that is not there.
            debug!(attachment_id, "Refused a T3 attachment delete");
            Ok(())
        }
        Err(AttachmentStoreError::Failed(error)) => {
            warn!(%error, attachment_id, "Failed to delete a T3 attachment");
            Err("The attachment could not be deleted.".to_string())
        }
    }
}

/// Take the bytes for one minted attachment.
///
/// Read against the size the token declares: a client sending more is refused
/// at the byte it goes over, not after 50 MiB is held in memory, and a body
/// ending short is refused too — a truncated upload stored anyway would reach
/// the agent as a corrupt file.
pub(crate) async fn receive(
    State(state): State<UiRpcState>,
    Path(token): Path<String>,
    body: Body,
) -> Response {
    let Some(signing_key) = state.session_jwt_secret.as_deref() else {
        error!("Cannot accept a T3 attachment upload: no JWT secret is configured");
        return (StatusCode::SERVICE_UNAVAILABLE, "Uploads are unavailable.").into_response();
    };
    let claims = match verify(&token, signing_key) {
        Ok(claims) => claims,
        Err(error) => {
            warn!(%error, "Rejected a T3 attachment upload URL");
            return (
                StatusCode::UNAUTHORIZED,
                "This upload URL is not valid any more.",
            )
                .into_response();
        }
    };

    // The declaration was checked against the union's per-type maximum before
    // it was signed. This is the backstop for that, not the check itself.
    let declared_size = claims.size.clamp(0, MAX_FILE_BYTES);
    let mut received = Vec::with_capacity(declared_size.min(UPLOAD_PREALLOCATE_MAX) as usize);
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                debug!(%error, attachment_id = %claims.aid, "A T3 attachment upload broke off");
                return (StatusCode::BAD_REQUEST, "The upload did not finish.").into_response();
            }
        };
        if received.len() + chunk.len() > declared_size as usize {
            debug!(
                attachment_id = %claims.aid,
                declared = declared_size,
                "Refused a T3 attachment upload that ran past its declared size"
            );
            return (
                StatusCode::BAD_REQUEST,
                "The upload is larger than it declared.",
            )
                .into_response();
        }
        received.extend_from_slice(&chunk);
    }

    if received.len() as i64 != declared_size {
        debug!(
            attachment_id = %claims.aid,
            declared = declared_size,
            received = received.len(),
            "Refused a T3 attachment upload that ended short of its declared size"
        );
        return (
            StatusCode::BAD_REQUEST,
            "The upload is smaller than it declared.",
        )
            .into_response();
    }

    let viewer = ScopeContext::new(claims.sub, claims.adm);
    match state
        .attachments
        .store(
            &viewer,
            &claims.aid,
            DeclaredAttachment {
                filename: claims.name,
                mime_type: claims.mime,
            },
            received,
        )
        .await
    {
        Ok(()) => {
            debug!(attachment_id = %claims.aid, bytes = declared_size, "Stored a T3 attachment");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(AttachmentStoreError::Forbidden) => {
            (StatusCode::FORBIDDEN, "The upload was refused.").into_response()
        }
        Err(AttachmentStoreError::Failed(error)) => {
            error!(%error, attachment_id = %claims.aid, "Failed to store a T3 attachment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "The upload could not be stored.",
            )
                .into_response()
        }
    }
}

fn verify(token: &str, signing_key: &str) -> Result<UploadClaims, jsonwebtoken::errors::Error> {
    let mut validation = jsonwebtoken::Validation::default();
    validation.set_issuer(&[UPLOAD_TOKEN_ISSUER]);
    jsonwebtoken::decode::<UploadClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(signing_key.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
}

/// `ChatAttachmentId`'s alphabet and length
/// (`contracts/src/orchestration.ts:180-183`); keeps a hand-written id from
/// reaching the store as a path or a wildcard.
fn is_attachment_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::ui_rpc::UserRole;

    const SECRET: &str = "a-test-signing-secret";

    fn identity() -> UiRpcAuthIdentity {
        UiRpcAuthIdentity {
            user_id: uuid::Uuid::nil(),
            login: "someone".to_string(),
            role: UserRole::User,
            provider: "password".to_string(),
            group_id: None,
        }
    }

    fn minted(payload: Value) -> Value {
        create_upload_url(&identity(), Some(SECRET), &payload)
            .unwrap_or_else(|_| panic!("the payload should have minted"))
    }

    fn claims(minted: &Value) -> UploadClaims {
        let url = minted["relativeUrl"].as_str().expect("a url");
        let token = url
            .strip_prefix(&format!("{UPLOAD_ROUTE_PREFIX}/"))
            .expect("the url is under the upload prefix");
        verify(token, SECRET).expect("the token this server just signed")
    }

    fn invalid(payload: Value) -> String {
        match create_upload_url(&identity(), Some(SECRET), &payload) {
            Err(MintError::Invalid(message)) => message,
            Err(MintError::NoSigningKey) => panic!("a key was supplied"),
            Ok(minted) => panic!("expected a refusal, got {minted}"),
        }
    }

    /// The one relation the rest of the surface rests on: the id in the answer
    /// is the id the upload lands under, so `assets.createUrl` resolves it to a
    /// download with nothing added.
    #[test]
    fn a_minted_url_names_the_attachment_it_may_write() {
        let minted = minted(json!({
            "type": "file",
            "name": "report.pdf",
            "mimeType": "application/pdf",
            "sizeBytes": 2048
        }));
        let claims = claims(&minted);

        assert_eq!(claims.aid, minted["attachmentId"].as_str().expect("an id"));
        assert_eq!(claims.name, "report.pdf");
        assert_eq!(claims.mime, "application/pdf");
        assert_eq!(claims.size, 2048);
        assert_eq!(claims.sub, uuid::Uuid::nil().to_string());
        assert!(!claims.adm);
        assert!(
            minted["expiresAt"]
                .as_i64()
                .is_some_and(|at| at > Utc::now().timestamp_millis())
        );
    }

    /// `type` is `optionalKey` on the image member, so a client that predates
    /// files sends none — and gets the image member's rules.
    #[test]
    fn a_payload_without_a_type_is_an_image() {
        assert_eq!(
            claims(&minted(json!({
                "name": "shot.png",
                "mimeType": "image/png",
                "sizeBytes": 10
            })))
            .mime,
            "image/png"
        );
        assert!(
            invalid(json!({
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "sizeBytes": 10
            }))
            .contains("not an image type")
        );
    }

    /// The union's two members differ in their ceiling, and the boundary is
    /// where a limit is either right or off by one.
    #[test]
    fn each_member_is_capped_at_its_own_maximum() {
        let image = |size| json!({ "type": "image", "name": "a.png", "mimeType": "image/png", "sizeBytes": size });
        let file = |size| json!({ "type": "file", "name": "a.bin", "mimeType": "application/octet-stream", "sizeBytes": size });

        assert_eq!(
            claims(&minted(image(MAX_IMAGE_BYTES))).size,
            MAX_IMAGE_BYTES
        );
        assert!(invalid(image(MAX_IMAGE_BYTES + 1)).contains("between 1 and"));
        assert_eq!(claims(&minted(file(MAX_FILE_BYTES))).size, MAX_FILE_BYTES);
        assert!(invalid(file(MAX_FILE_BYTES + 1)).contains("between 1 and"));

        // An image over the image ceiling is refused even though a file that
        // size would be accepted: the member decides the limit, not the bytes.
        assert!(invalid(image(MAX_FILE_BYTES)).contains("between 1 and"));
    }

    /// Every field the union constrains, at the edge the client's own schema
    /// would have caught.
    #[test]
    fn a_payload_outside_the_union_is_refused_field_by_field() {
        let file = |name: &str, mime: &str, size: i64| json!({ "type": "file", "name": name, "mimeType": mime, "sizeBytes": size });

        assert!(invalid(file("  ", "application/pdf", 10)).contains("`name`"));
        assert!(
            invalid(file(&"a".repeat(NAME_MAX_CHARS + 1), "application/pdf", 10))
                .contains("`name`")
        );
        assert!(invalid(file("a.pdf", "", 10)).contains("`mimeType`"));
        assert!(invalid(file("a.pdf", &"a".repeat(MIME_MAX_CHARS + 1), 10)).contains("`mimeType`"));
        // The contract's minimum is one byte, on both members.
        assert!(invalid(file("a.pdf", "application/pdf", 0)).contains("between 1 and"));
        assert!(invalid(file("a.pdf", "application/pdf", -1)).contains("between 1 and"));
        assert!(
            invalid(json!({ "type": "spreadsheet", "name": "a", "mimeType": "text/csv", "sizeBytes": 1 }))
                .contains("not an attachment type")
        );
        // A name at the limit is kept: the check is on the count, not on bytes.
        assert_eq!(
            claims(&minted(file(
                &"ä".repeat(NAME_MAX_CHARS),
                "application/pdf",
                10
            )))
            .name,
            "ä".repeat(NAME_MAX_CHARS)
        );
    }

    /// Mime types are matched case-insensitively and stored lowered, because
    /// what a file picker reports is not something this side chooses.
    #[test]
    fn a_mime_type_is_matched_and_stored_lowered() {
        assert_eq!(
            claims(&minted(json!({
                "type": "image",
                "name": "a.png",
                "mimeType": "IMAGE/PNG",
                "sizeBytes": 10
            })))
            .mime,
            "image/png"
        );
    }

    /// A deployment with no JWT secret cannot sign anything, and the contract
    /// has a name for that. It is checked after the payload, so a malformed
    /// request is still called malformed.
    #[test]
    fn no_signing_key_is_the_failure_the_contract_names() {
        assert!(matches!(
            create_upload_url(
                &identity(),
                None,
                &json!({ "type": "file", "name": "a.pdf", "mimeType": "application/pdf", "sizeBytes": 1 })
            ),
            Err(MintError::NoSigningKey)
        ));
        assert!(matches!(
            create_upload_url(&identity(), None, &json!({ "type": "file" })),
            Err(MintError::Invalid(_))
        ));
    }

    /// The token is the credential, so a URL from another deployment, or one
    /// whose claims were edited, writes nothing.
    #[test]
    fn a_token_signed_with_another_secret_is_not_accepted() {
        let url = minted(json!({
            "type": "file",
            "name": "a.pdf",
            "mimeType": "application/pdf",
            "sizeBytes": 1
        }));
        let token = url["relativeUrl"]
            .as_str()
            .and_then(|url| url.strip_prefix(&format!("{UPLOAD_ROUTE_PREFIX}/")))
            .expect("a token");

        assert!(verify(token, "a-different-secret").is_err());
        assert!(verify(&format!("{token}x"), SECRET).is_err());
    }

    /// What `attachments.delete` will hand the store. A path, a wildcard or an
    /// empty string is not an id and never reaches it.
    #[test]
    fn only_an_id_shaped_like_the_contracts_reaches_the_store() {
        assert!(is_attachment_id("6b1f7b2e-0000-4000-8000-000000000001"));
        assert!(is_attachment_id("a_b-C9"));
        assert!(!is_attachment_id(""));
        assert!(!is_attachment_id("../secrets"));
        assert!(!is_attachment_id("a/b"));
        assert!(!is_attachment_id("a b"));
        assert!(!is_attachment_id(&"a".repeat(129)));
    }
}
