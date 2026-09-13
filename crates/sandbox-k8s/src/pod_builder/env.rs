//! Environment values a pod spec carries, and the redaction applied before a
//! URL becomes one.

use super::*;

/// Create an `EnvVar`.
pub(super) fn env_var(name: &str, value: &str) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value: Some(value.to_string()),
        ..Default::default()
    }
}

/// Strip any embedded userinfo (credentials) from an HTTP(S) URL.
///
/// `https://user:pat@host/owner/repo.git` → `https://host/owner/repo.git`;
/// non-HTTP(S) forms (e.g. scp-like SSH `git@host:path`) are unchanged since
/// their user isn't a secret. Keeps a PAT/password out of the pod spec
/// (annotation + `GIT_CLONE_URL`) and thus out of etcd.
pub(super) fn redact_url_userinfo(url: &str) -> String {
    for scheme in ["https://", "http://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            // Look only within the authority (before the first '/') for userinfo.
            let authority_end = rest.find('/').unwrap_or(rest.len());
            if let Some(at) = rest[..authority_end].rfind('@') {
                return format!("{scheme}{}", &rest[at + 1..]);
            }
            return url.to_string();
        }
    }
    url.to_string()
}

pub(super) fn chrono_now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
