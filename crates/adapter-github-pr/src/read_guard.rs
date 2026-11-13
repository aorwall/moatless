//! What every [`GithubPrApi`](crate::api::GithubPrApi) one Adapter mints
//! shares across requests: each credential's spent quota, and the pull
//! requests it read moments ago.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::api::PrInfo;

/// How long a pull request read is answered from memory.
pub(crate) const PR_TTL: Duration = Duration::from_secs(20);
/// The hold when GitHub refuses for its rate limit without saying until when.
const UNTIMED_HOLD: chrono::TimeDelta = chrono::TimeDelta::seconds(60);
/// Pull-request slots past which idle and stale ones are dropped; slots in
/// use or fresh stay, so this is a soft limit rather than a cap.
const PR_SLOTS: usize = 256;

/// A credential, by a hash of its token so no token is held twice.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TokenKey(u64);

impl TokenKey {
    pub(crate) fn of(token: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        Self(hasher.finish())
    }
}

type PrSlot = Arc<tokio::sync::Mutex<Option<(Instant, PrInfo)>>>;

#[derive(Default)]
pub(crate) struct ReadGuard {
    held_until: Mutex<HashMap<TokenKey, DateTime<Utc>>>,
    pulls: Mutex<HashMap<(TokenKey, String, String, u64), PrSlot>>,
}

impl ReadGuard {
    /// When `token`'s quota returns, while GitHub is still refusing it.
    pub(crate) fn held_until(&self, token: TokenKey, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut held = self
            .held_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match held.get(&token) {
            Some(until) if *until > now => Some(*until),
            Some(_) => {
                held.remove(&token);
                None
            }
            None => None,
        }
    }

    /// Hold `token`'s requests until `retry_at`, or [`UNTIMED_HOLD`] from
    /// `now`, keeping a later hold already in place. True when it was not
    /// already held, so the caller logs once.
    pub(crate) fn hold(
        &self,
        token: TokenKey,
        retry_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> (bool, DateTime<Utc>) {
        let until = retry_at.unwrap_or(now + UNTIMED_HOLD);
        let mut held = self
            .held_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let previous = held.get(&token).copied().filter(|previous| *previous > now);
        // The later end wins: an untimed refusal racing a timed one must not
        // cut the timed hold short.
        let until = previous.map_or(until, |previous| previous.max(until));
        held.insert(token, until);
        (previous.is_none(), until)
    }

    /// The slot that coalesces reads of one pull request with one credential:
    /// a caller holds its lock across the read, so a concurrent one waits and
    /// then answers from it.
    pub(crate) fn pr_slot(&self, token: TokenKey, owner: &str, repo: &str, number: u64) -> PrSlot {
        let mut pulls = self.pulls.lock().unwrap_or_else(PoisonError::into_inner);
        if pulls.len() >= PR_SLOTS {
            pulls.retain(|_, slot| {
                Arc::strong_count(slot) > 1
                    || slot.try_lock().is_ok_and(|read| {
                        read.as_ref().is_some_and(|(at, _)| at.elapsed() < PR_TTL)
                    })
            });
        }
        pulls
            .entry((token, owner.to_string(), repo.to_string(), number))
            .or_default()
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_ends_at_its_reset_and_logs_only_when_it_starts() {
        let guard = ReadGuard::default();
        let token = TokenKey::of("ghp_a");
        let now = DateTime::from_timestamp(1_793_000_000, 0).unwrap();
        let reset = now + chrono::TimeDelta::minutes(10);

        assert_eq!(guard.hold(token, Some(reset), now), (true, reset));
        assert_eq!(guard.hold(token, Some(reset), now), (false, reset));
        assert_eq!(guard.held_until(token, now), Some(reset));
        assert_eq!(guard.held_until(TokenKey::of("ghp_b"), now), None);
        assert_eq!(guard.held_until(token, reset), None);
    }

    #[test]
    fn an_untimed_refusal_holds_for_a_minute() {
        let guard = ReadGuard::default();
        let token = TokenKey::of("ghp_a");
        let now = DateTime::from_timestamp(1_793_000_000, 0).unwrap();

        let (_, until) = guard.hold(token, None, now);

        assert_eq!(until, now + UNTIMED_HOLD);
    }

    #[test]
    fn an_untimed_refusal_does_not_shorten_a_timed_hold() {
        let guard = ReadGuard::default();
        let token = TokenKey::of("ghp_a");
        let now = DateTime::from_timestamp(1_793_000_000, 0).unwrap();
        let reset = now + chrono::TimeDelta::minutes(10);

        guard.hold(token, Some(reset), now);

        assert_eq!(guard.hold(token, None, now), (false, reset));
    }

    mod over_http {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::{IntoResponse, Response};
        use axum::routing::get;
        use axum::{Json, Router};

        use super::*;
        use crate::api::{GithubPrApi, http_client};
        use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
        use std::time::Duration;

        async fn serve(app: Router) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            format!("http://{address}")
        }

        /// The reset is an hour after any `now` the test runs at.
        async fn spent(State(reads): State<Arc<AtomicUsize>>) -> Response {
            reads.fetch_add(1, Ordering::SeqCst);
            let reset = (chrono::Utc::now() + chrono::TimeDelta::hours(1))
                .timestamp()
                .to_string();
            (
                StatusCode::FORBIDDEN,
                [
                    ("x-ratelimit-remaining", "0".to_string()),
                    ("x-ratelimit-reset", reset),
                ],
                r#"{"message":"API rate limit exceeded for user ID 609489."}"#,
            )
                .into_response()
        }

        async fn pull(State(reads): State<Arc<AtomicUsize>>) -> Json<serde_json::Value> {
            reads.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Json(serde_json::json!({
                "number": 42,
                "title": "Add widgets",
                "state": "open",
                "head": {"ref": "feature", "sha": "abc1234"},
                "base": {"ref": "main", "sha": "def4567"}
            }))
        }

        #[tokio::test]
        async fn a_spent_quota_holds_every_client_of_that_token_until_its_reset() {
            let reads = Arc::new(AtomicUsize::new(0));
            let base = serve(
                Router::new()
                    .route("/repos/acme/widgets/pulls/42", get(spent))
                    .with_state(reads.clone()),
            )
            .await;
            let guard = Arc::new(ReadGuard::default());
            let api = |token: &str| {
                GithubPrApi::sharing(
                    http_client(moatless_core::config::HttpConfig::default().github),
                    token,
                    &base,
                    guard.clone(),
                )
            };

            let first = api("ghp_test").get_pr("acme", "widgets", 42).await;
            let held = api("ghp_test").get_pr("acme", "widgets", 42).await;
            let other = api("ghp_other").get_pr("acme", "widgets", 42).await;

            for err in [first, held, other] {
                assert!(
                    matches!(
                        err,
                        Err(AdapterError::Shared(SharedAdapterError::RateLimited {
                            retry_at: Some(_),
                            ..
                        }))
                    ),
                    "got {err:?}"
                );
            }
            assert_eq!(
                reads.load(Ordering::SeqCst),
                2,
                "the held token's second read never reached GitHub"
            );
        }

        #[tokio::test]
        async fn recent_reads_of_one_pull_request_share_one_request() {
            let reads = Arc::new(AtomicUsize::new(0));
            let base = serve(
                Router::new()
                    .route("/repos/acme/widgets/pulls/42", get(pull))
                    .with_state(reads.clone()),
            )
            .await;
            let guard = Arc::new(ReadGuard::default());
            let api = |token: &str| {
                GithubPrApi::sharing(
                    http_client(moatless_core::config::HttpConfig::default().github),
                    token,
                    &base,
                    guard.clone(),
                )
            };
            let (a, b) = (api("ghp_test"), api("ghp_test"));

            let (first, second) = tokio::join!(
                a.get_pr_recent("acme", "widgets", 42),
                b.get_pr_recent("acme", "widgets", 42)
            );
            let later = a.get_pr_recent("acme", "widgets", 42).await;
            assert_eq!(reads.load(Ordering::SeqCst), 1);

            api("ghp_other")
                .get_pr_recent("acme", "widgets", 42)
                .await
                .expect("another token reads for itself");
            a.get_pr("acme", "widgets", 42).await.expect("a fresh read");
            for pr in [first, second, later] {
                assert_eq!(pr.expect("a read").head.sha, "abc1234");
            }
            assert_eq!(reads.load(Ordering::SeqCst), 3);
        }
    }
}
