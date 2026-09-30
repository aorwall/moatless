//! TypeSafe Jev: the `/v1/systemone` wire shape and its retry policy.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use moatless_api_schemas::decisions::{DecisionOption, DecisionUsage};
use moatless_core::config::JevConfig;
use moatless_types::http::HttpTimeouts;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use tokio::time::Instant;

use crate::provider::{ProviderError, ProviderPick, Question};

pub(crate) const PROVIDER: &str = "jev";
/// TypeSafe's documented budget for one call's state plus questions.
pub(crate) const MAX_INPUT_CHARS: usize = 60_000;
/// TypeSafe's recommended thresholds for Jev's calibrated confidence.
pub(crate) const ACT_AT: f64 = 0.75;
pub(crate) const CONFIRM_AT: f64 = 0.45;

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);
const RETRY_AFTER_CAP: Duration = Duration::from_secs(4);
/// One entry per retry, so its length is the retry limit.
const BACKOFF: [Duration; 2] = [Duration::from_millis(250), Duration::from_secs(1)];
/// TypeSafe's "overloaded", distinct from the 429 rate limit.
const OVERLOADED: u16 = 529;

pub(crate) struct JevClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl fmt::Debug for JevClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevClient")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl JevClient {
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub(crate) fn new(config: &JevConfig, timeouts: HttpTimeouts) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .user_agent(moatless_core::USER_AGENT)
            .build()
            .expect("the TLS backend initialises");
        Self {
            http,
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            model: config.model.clone(),
        }
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    /// Retries only 429 and 529, and only while the backoff fits before
    /// `deadline`; each attempt is also capped at [`ATTEMPT_TIMEOUT`].
    pub(crate) async fn pick(
        &self,
        question: &Question<'_>,
        deadline: Instant,
    ) -> Result<ProviderPick, ProviderError> {
        let url = format!("{}/v1/systemone", self.base_url);
        let body = JevRequest {
            model: &self.model,
            state: question.state,
            questions: JevQuestions {
                q: JevQuestion {
                    kind: "choice",
                    instructions: question.text,
                    criteria: Criteria(question.options),
                },
            },
        };
        let mut attempts = 0;
        loop {
            attempts += 1;
            let remaining = deadline.saturating_duration_since(Instant::now());
            let response = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .timeout(remaining.min(ATTEMPT_TIMEOUT))
                .json(&body)
                .send()
                .await
                .map_err(ProviderError::Transport)?;
            let status = response.status();
            if status.is_success() {
                let bytes = response.bytes().await.map_err(ProviderError::Transport)?;
                return parse_answer(&bytes, attempts);
            }
            // The error body is never read: it may echo the state back.
            match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    return Err(ProviderError::Auth {
                        status: status.as_u16(),
                    });
                }
                StatusCode::UNPROCESSABLE_ENTITY => return Err(ProviderError::Rejected),
                _ if status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == OVERLOADED => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    let Some(delay) =
                        retry_delay(attempts, retry_after(response.headers()), remaining)
                    else {
                        return Err(ProviderError::Overloaded {
                            status: status.as_u16(),
                            attempts,
                        });
                    };
                    crate::metrics::record_retry(PROVIDER, status.as_u16());
                    tokio::time::sleep(delay).await;
                }
                _ => {
                    return Err(ProviderError::Upstream {
                        status: status.as_u16(),
                    });
                }
            }
        }
    }
}

/// The wait before retry number `attempts`, or `None` when the retries are
/// spent or the wait would not end before `remaining` runs out.
pub(crate) fn retry_delay(
    attempts: u32,
    retry_after: Option<Duration>,
    remaining: Duration,
) -> Option<Duration> {
    let backoff = *BACKOFF.get(usize::try_from(attempts).ok()?.checked_sub(1)?)?;
    let delay = retry_after.map_or(backoff, |wait| wait.min(RETRY_AFTER_CAP));
    (delay < remaining).then_some(delay)
}

/// Only the delay-seconds form: TypeSafe sends no HTTP dates.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

fn parse_answer(body: &[u8], attempts: u32) -> Result<ProviderPick, ProviderError> {
    let response: JevResponse = serde_json::from_slice(body).map_err(ProviderError::Decode)?;
    let answer = response
        .answers
        .q
        .ok_or(ProviderError::Malformed("no answer to the question"))?;
    let choice = answer
        .choice
        .ok_or(ProviderError::Malformed("answer has no choice"))?;
    Ok(ProviderPick {
        choice,
        // Out of range is dropped rather than trusted, so it bands as `escalate`.
        confidence: answer.confidence.filter(|c| (0.0..=1.0).contains(c)),
        probabilities: answer.probabilities.into_iter().collect(),
        usage: DecisionUsage {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        },
        attempts,
    })
}

#[derive(Serialize)]
struct JevRequest<'a> {
    model: &'a str,
    state: &'a str,
    questions: JevQuestions<'a>,
}

#[derive(Serialize)]
struct JevQuestions<'a> {
    q: JevQuestion<'a>,
}

#[derive(Serialize)]
struct JevQuestion<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    instructions: &'a str,
    criteria: Criteria<'a>,
}

/// `{name: description}` in the caller's order; a bare name describes itself.
struct Criteria<'a>(&'a [DecisionOption]);

impl Serialize for Criteria<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for option in self.0 {
            let description = option.description.as_deref().unwrap_or(&option.name);
            map.serialize_entry(&option.name, description)?;
        }
        map.end()
    }
}

#[derive(Deserialize)]
struct JevResponse {
    answers: JevAnswers,
    #[serde(default)]
    usage: JevUsage,
}

#[derive(Deserialize)]
struct JevAnswers {
    q: Option<JevAnswer>,
}

#[derive(Deserialize)]
struct JevAnswer {
    choice: Option<String>,
    confidence: Option<f64>,
    #[serde(default)]
    probabilities: HashMap<String, f64>,
}

#[derive(Default, Deserialize)]
struct JevUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}
