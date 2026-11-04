//! The MAC that authenticates the backend to a sandbox's gRPC server.
//!
//! The backend derives one key per Sandbox from `TASK_TOKEN_SECRET` and hands
//! it to the pod as `SANDBOX_GRPC_KEY`; the secret itself never reaches a pod,
//! where it would mint any user's token. Each call carries
//! `x-sandbox-auth: <unix secs>.<hex HMAC(key, unix secs)>`.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tonic::service::Interceptor;
use tonic::{Request, Status};

type HmacSha256 = Hmac<Sha256>;

/// The metadata key a backend call carries its MAC in.
pub const SANDBOX_AUTH_HEADER: &str = "x-sandbox-auth";

/// The env var a pod reads its key from.
pub const SANDBOX_GRPC_KEY_ENV: &str = "SANDBOX_GRPC_KEY";

/// The env var naming whether a pod refuses an unauthenticated call
/// (`enforce`) or only logs it (anything else).
pub const SANDBOX_GRPC_AUTH_ENV: &str = "SANDBOX_GRPC_AUTH";

/// How far a call's timestamp may sit from the pod's clock, in seconds.
const MAX_SKEW_SECS: u64 = 60;

/// The key for `sandbox_id`, as hex. Deterministic, so every backend replica
/// derives the one the pod was given.
pub fn derive_key(secret: &str, sandbox_id: &str) -> String {
    let label = format!("sandbox-grpc/v1/{sandbox_id}");
    hex::encode(
        hmac(secret.as_bytes())
            .chain_update(label)
            .finalize()
            .into_bytes(),
    )
}

/// Why a call failed authentication.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("no x-sandbox-auth header")]
    Missing,
    #[error("malformed x-sandbox-auth header")]
    Malformed,
    #[error("x-sandbox-auth timestamp outside the allowed skew")]
    Stale,
    #[error("x-sandbox-auth MAC does not match")]
    Mismatch,
}

fn hmac(key: &[u8]) -> HmacSha256 {
    #[expect(clippy::expect_used, reason = "HMAC accepts a key of any length")]
    HmacSha256::new_from_slice(key).expect("HMAC takes any key length")
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

/// The header value for a call made at `now` (unix seconds).
pub fn sign(key: &str, now: u64) -> String {
    let ts = now.to_string();
    let mac = hmac(key.as_bytes())
        .chain_update(&ts)
        .finalize()
        .into_bytes();
    format!("{ts}.{}", hex::encode(mac))
}

/// Checks `header` against `key` at `now` (unix seconds).
pub fn verify(key: &str, header: Option<&str>, now: u64) -> Result<(), AuthError> {
    let header = header.ok_or(AuthError::Missing)?;
    let (ts, mac_hex) = header.split_once('.').ok_or(AuthError::Malformed)?;
    let issued: u64 = ts.parse().map_err(|_| AuthError::Malformed)?;
    let given = hex::decode(mac_hex).map_err(|_| AuthError::Malformed)?;
    if issued.abs_diff(now) > MAX_SKEW_SECS {
        return Err(AuthError::Stale);
    }
    hmac(key.as_bytes())
        .chain_update(ts)
        .verify_slice(&given)
        .map_err(|_| AuthError::Mismatch)
}

/// Signs every outgoing call with the Sandbox's key; with no key, a no-op, for
/// a deployment that mints no tokens.
#[derive(Clone, Default)]
pub struct SignCalls {
    key: Option<String>,
}

impl SignCalls {
    pub fn new(key: Option<String>) -> Self {
        Self { key }
    }
}

impl Interceptor for SignCalls {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(key) = &self.key {
            let value = sign(key, now_secs())
                .parse()
                .map_err(|_| Status::internal("unencodable x-sandbox-auth"))?;
            request.metadata_mut().insert(SANDBOX_AUTH_HEADER, value);
        }
        Ok(request)
    }
}

/// Checks every incoming call against the pod's key.
///
/// A loopback peer passes unchecked: `moat cmd` runs inside the pod. With no
/// key every call passes, since a pod from an older backend was given none.
/// Without `enforce` a failing call is logged and served.
#[derive(Clone)]
pub struct VerifyCalls {
    key: Option<String>,
    enforce: bool,
}

impl VerifyCalls {
    pub fn new(key: Option<String>, enforce: bool) -> Self {
        Self { key, enforce }
    }
}

impl Interceptor for VerifyCalls {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(key) = &self.key else {
            return Ok(request);
        };
        if request
            .remote_addr()
            .is_some_and(|addr| addr.ip().is_loopback())
        {
            return Ok(request);
        }
        let header = request
            .metadata()
            .get(SANDBOX_AUTH_HEADER)
            .and_then(|value| value.to_str().ok());
        match verify(key, header, now_secs()) {
            Ok(()) => Ok(request),
            Err(error) => {
                tracing::warn!(
                    %error,
                    peer = ?request.remote_addr(),
                    enforced = self.enforce,
                    "Unauthenticated sandbox gRPC call"
                );
                if self.enforce {
                    Err(Status::unauthenticated(error.to_string()))
                } else {
                    Ok(request)
                }
            }
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "test fixtures must succeed")]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn a_signed_call_verifies_within_the_skew_and_not_past_it() {
        let key = derive_key("secret", "sandbox-1");
        assert_eq!(verify(&key, Some(&sign(&key, NOW)), NOW + 59), Ok(()));
        assert_eq!(
            verify(&key, Some(&sign(&key, NOW)), NOW + 61),
            Err(AuthError::Stale)
        );
        assert_eq!(
            verify(&key, Some(&sign(&key, NOW + 61)), NOW),
            Err(AuthError::Stale)
        );
    }

    #[test]
    fn another_sandboxs_key_does_not_verify() {
        let key = derive_key("secret", "sandbox-1");
        let other = derive_key("secret", "sandbox-2");
        assert_ne!(key, other);
        assert_eq!(
            verify(&key, Some(&sign(&other, NOW)), NOW),
            Err(AuthError::Mismatch)
        );
    }

    #[test]
    fn a_missing_or_malformed_header_is_refused() {
        let key = derive_key("secret", "sandbox-1");
        assert_eq!(verify(&key, None, NOW), Err(AuthError::Missing));
        assert_eq!(verify(&key, Some("nodot"), NOW), Err(AuthError::Malformed));
        assert_eq!(verify(&key, Some("x.abcd"), NOW), Err(AuthError::Malformed));
        assert_eq!(
            verify(&key, Some(&format!("{NOW}.zz")), NOW),
            Err(AuthError::Malformed)
        );
    }

    #[test]
    fn the_server_interceptor_enforces_only_when_told_to() {
        let key = derive_key("secret", "sandbox-1");
        let unsigned = || Request::new(());

        assert!(
            VerifyCalls::new(Some(key.clone()), false)
                .call(unsigned())
                .is_ok()
        );
        assert_eq!(
            VerifyCalls::new(Some(key.clone()), true)
                .call(unsigned())
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        assert!(VerifyCalls::new(None, true).call(unsigned()).is_ok());

        let signed = SignCalls::new(Some(key.clone())).call(unsigned()).unwrap();
        assert!(VerifyCalls::new(Some(key), true).call(signed).is_ok());
    }
}
