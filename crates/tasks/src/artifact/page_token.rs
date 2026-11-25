//! The credential an Artifact page that accepts data saves records with.
//!
//! A page is served in an opaque origin (see `handlers::serve_artifact_page`),
//! so it sends no session cookie. The serve route signs one of these for the
//! reader it serves and writes it into the page; the record routes under
//! [`PAGE_RECORDS_ROUTE`] trust the token, never a cookie.

use serde::{Deserialize, Serialize};

/// The page's record route; `{token}` is the page token.
///
/// The `#[utoipa::path]` attributes, `data_script.js` and the auth-middleware
/// exemption in `moatless_identity::auth::middleware` each hold a copy of this
/// literal: rename all four together, or the page's saves get a 401. The
/// request span's redaction in `moatless_backend::http_trace` holds its prefix.
pub const PAGE_RECORDS_ROUTE: &str = "/api/artifact-data/{token}/records";

/// The page's read-only route for an attached collection; copies as for
/// [`PAGE_RECORDS_ROUTE`].
pub const PAGE_COLLECTION_ROUTE: &str = "/api/artifact-data/{token}/collections/{name}/records";

/// Long enough for a page left open through a working day.
const TTL_SECS: i64 = 12 * 60 * 60;

const ISSUER: &str = "moatless-backend/artifact-page";

/// Issuer before the rename to [`ISSUER`]. Still accepted for one [`TTL_SECS`],
/// so open pages keep working through the rollout.
const LEGACY_ISSUER: &str = "moatless-vibe-backend/artifact-page";

/// What a token authorizes: one reader, on one Artifact, until `exp`.
///
/// `adm` and `sub` are frozen at signing. A reader demoted from admin keeps
/// admin-scope reads of this one Artifact until the token expires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageClaims {
    /// The reader's user id; records they save are attributed to it.
    pub sub: String,
    /// Whether the reader is an admin, so the access recheck uses the scope
    /// the serve route had.
    pub adm: bool,
    /// The reader's login, for the agent's `<from>` header.
    pub lgn: String,
    /// The Artifact id.
    pub aid: String,
    pub exp: u64,
    iss: String,
}

/// Sign a token for `user_id` on `artifact_id`.
pub fn mint(
    signing_key: &str,
    user_id: &str,
    admin: bool,
    login: &str,
    artifact_id: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = chrono::Utc::now().timestamp() + TTL_SECS;
    let claims = PageClaims {
        sub: user_id.to_string(),
        adm: admin,
        lgn: login.to_string(),
        aid: artifact_id.to_string(),
        exp: u64::try_from(exp).unwrap_or_default(),
        iss: ISSUER.to_string(),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(signing_key.as_bytes()),
    )
}

/// The claims of a token this deployment signed and that has not expired.
pub fn verify(token: &str, signing_key: &str) -> Result<PageClaims, jsonwebtoken::errors::Error> {
    let mut validation = jsonwebtoken::Validation::default();
    validation.set_issuer(&[ISSUER, LEGACY_ISSUER]);
    jsonwebtoken::decode::<PageClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(signing_key.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_token_verifies_with_its_key_only() {
        let token = mint("key-a", "user-1", false, "alice", "art-1").unwrap();
        let claims = verify(&token, "key-a").unwrap();
        assert_eq!(
            (
                claims.sub.as_str(),
                claims.lgn.as_str(),
                claims.aid.as_str()
            ),
            ("user-1", "alice", "art-1")
        );
        assert!(!claims.adm);
        assert!(verify(&token, "key-b").is_err());
    }

    #[test]
    fn a_token_from_another_issuer_is_refused() {
        let claims = PageClaims {
            sub: "user-1".to_string(),
            adm: true,
            lgn: "alice".to_string(),
            aid: "art-1".to_string(),
            exp: u64::try_from(chrono::Utc::now().timestamp() + 60).unwrap(),
            iss: "someone-else".to_string(),
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"key-a"),
        )
        .unwrap();
        assert!(verify(&token, "key-a").is_err());
    }
}
