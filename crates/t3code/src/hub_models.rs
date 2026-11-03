//! The models the deployment's `CLIProxyAPI` hub serves, as Pi's model list.
//!
//! Pi runs any `anthropic/<id>` or `openai/<id>` the gateway can reach, so the
//! hub's own `GET /v1/models` is a better list than a static catalog.

use std::sync::Arc;
use std::time::{Duration, Instant};

use moatless_api_schemas::agent::AgentModel;
use moatless_types::http::HttpTimeouts;
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::warn;

/// Long enough that opening the picker does not call the hub each time, short
/// enough that a model the hub gains shows up within minutes.
const CACHE_TTL: Duration = Duration::from_mins(5);

/// Not chat models: a Pi turn on one of these fails. Matched as substrings, so a
/// future chat id containing one is hidden.
const NON_CHAT_MARKERS: &[&str] = &[
    "image",
    "embed",
    "tts",
    "whisper",
    "audio",
    "realtime",
    "moderation",
    "transcribe",
    "dall-e",
    "codex-auto-review",
];

/// The hub's model list, cached.
pub struct HubModels {
    url: String,
    key: String,
    client: reqwest::Client,
    /// The hub's last usable answer, unfiltered: callers differ in their static list.
    cached: Mutex<Option<(Instant, Value)>>,
}

impl HubModels {
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(url: &str, key: String, timeouts: HttpTimeouts) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(moatless_core::USER_AGENT)
            .build()
            .expect("the TLS backend initialises");
        Arc::new(Self {
            url: url.trim_end_matches('/').to_string(),
            key,
            client,
            cached: Mutex::new(None),
        })
    }

    /// Pi's models as `provider/model`; empty when the hub never gave a usable
    /// answer. A failed refresh serves the last answer.
    ///
    /// The lock is held across the request, so while the hub hangs every caller
    /// waits out the client timeout.
    pub async fn models(&self, static_models: &[AgentModel]) -> Vec<AgentModel> {
        let mut cached = self.cached.lock().await;
        if let Some((at, body)) = cached.as_ref() {
            if at.elapsed() < CACHE_TTL {
                return pi_models(body, static_models);
            }
        }
        match self.fetch().await {
            Ok(body) if !pi_models(&body, &[]).is_empty() => {
                *cached = Some((Instant::now(), body.clone()));
                return pi_models(&body, static_models);
            }
            Ok(_) => warn!("the hub's model list held no usable chat models"),
            Err(error) => warn!(%error, "reading the hub's model list failed"),
        }
        // Retry after the TTL, not on every request, while the hub is down.
        match cached.as_mut() {
            Some((at, body)) => {
                *at = Instant::now();
                pi_models(body, static_models)
            }
            None => Vec::new(),
        }
    }

    async fn fetch(&self) -> Result<Value, reqwest::Error> {
        self.client
            .get(format!("{}/v1/models", self.url))
            .bearer_auth(&self.key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
}

/// Chat models of the hub's `data` rows, newest first, with `static_models`
/// leading under their own labels.
///
/// The hub lists a dated id (`claude-sonnet-4-20250514`) beside its alias; only
/// the alias is kept.
fn pi_models(body: &Value, static_models: &[AgentModel]) -> Vec<AgentModel> {
    let mut rows: Vec<(i64, String)> = Vec::new();
    for row in body["data"].as_array().into_iter().flatten() {
        let (Some(id), Some(vendor)) = (row["id"].as_str(), row["owned_by"].as_str()) else {
            continue;
        };
        if !matches!(vendor, "anthropic" | "openai") || is_non_chat(id) {
            continue;
        }
        let slug = format!("{vendor}/{id}");
        if !rows.iter().any(|(_, existing)| *existing == slug) {
            rows.push((row["created"].as_i64().unwrap_or(0), slug));
        }
    }
    let aliases: Vec<String> = rows.iter().map(|(_, slug)| slug.clone()).collect();
    rows.retain(|(_, slug)| {
        undated(slug).is_none_or(|alias| !aliases.iter().any(|other| other == alias))
    });
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));

    let mut models: Vec<AgentModel> = static_models
        .iter()
        .filter(|model| rows.iter().any(|(_, slug)| *slug == model.id))
        .cloned()
        .collect();
    for (_, slug) in rows {
        if !models.iter().any(|model| model.id == slug) {
            let label = slug.split_once('/').map_or(slug.as_str(), |(_, id)| id);
            models.push(AgentModel {
                label: label.to_string(),
                id: slug,
            });
        }
    }
    models
}

fn is_non_chat(id: &str) -> bool {
    NON_CHAT_MARKERS.iter().any(|marker| id.contains(marker))
}

/// `id` without a trailing `-YYYYMMDD`, when it has one.
fn undated(id: &str) -> Option<&str> {
    let (alias, date) = id.rsplit_once('-')?;
    (date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit())).then_some(alias)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn static_models() -> Vec<AgentModel> {
        vec![
            AgentModel {
                id: "anthropic/claude-sonnet-5-5".into(),
                label: "Sonnet".into(),
            },
            AgentModel {
                id: "openai/gpt-6.1-sol".into(),
                label: "GPT 6.1 Sol".into(),
            },
            AgentModel {
                id: "anthropic/claude-haiku-5-5".into(),
                label: "Haiku".into(),
            },
        ]
    }

    fn ids(models: &[AgentModel]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn known_models_lead_under_their_catalog_labels_and_new_ones_follow_newest_first() {
        let body = json!({"data": [
            {"id": "claude-opus-4-8", "owned_by": "anthropic", "created": 10},
            {"id": "gpt-6.1-sol", "owned_by": "openai", "created": 1},
            {"id": "claude-sonnet-5-5", "owned_by": "anthropic", "created": 2},
            {"id": "gpt-6.1-terra", "owned_by": "openai", "created": 20},
        ]});

        let models = pi_models(&body, &static_models());

        assert_eq!(
            ids(&models),
            [
                "anthropic/claude-sonnet-5-5",
                "openai/gpt-6.1-sol",
                "openai/gpt-6.1-terra",
                "anthropic/claude-opus-4-8"
            ]
        );
        assert_eq!(models[0].label, "Sonnet");
        assert_eq!(models[2].label, "gpt-6.1-terra");
    }

    #[test]
    fn a_catalog_model_the_hub_does_not_serve_is_not_published() {
        let body = json!({"data": [
            {"id": "gpt-6.1-sol", "owned_by": "openai", "created": 1},
        ]});

        assert_eq!(
            ids(&pi_models(&body, &static_models())),
            ["openai/gpt-6.1-sol"]
        );
    }

    #[test]
    fn non_chat_models_and_other_vendors_are_left_out() {
        let body = json!({"data": [
            {"id": "gpt-image-2", "owned_by": "openai", "created": 1},
            {"id": "codex-auto-review", "owned_by": "openai", "created": 1},
            {"id": "gemini-3", "owned_by": "google", "created": 1},
            {"owned_by": "openai", "created": 1},
            {"id": "gpt-6-luna", "owned_by": "openai", "created": 1},
        ]});

        assert_eq!(ids(&pi_models(&body, &[])), ["openai/gpt-6-luna"]);
    }

    #[test]
    fn a_dated_id_is_dropped_only_when_its_alias_is_listed() {
        let body = json!({"data": [
            {"id": "claude-sonnet-4-20250514", "owned_by": "anthropic", "created": 3},
            {"id": "claude-sonnet-4", "owned_by": "anthropic", "created": 2},
            {"id": "claude-opus-4-1-20250805", "owned_by": "anthropic", "created": 1},
        ]});

        assert_eq!(
            ids(&pi_models(&body, &[])),
            [
                "anthropic/claude-sonnet-4",
                "anthropic/claude-opus-4-1-20250805"
            ]
        );
    }

    #[test]
    fn an_answer_without_rows_publishes_nothing() {
        assert!(pi_models(&json!({"error": "x"}), &static_models()).is_empty());
    }

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FakeHub {
        url: String,
        hits: Arc<AtomicUsize>,
        down: Arc<AtomicBool>,
    }

    async fn fake_hub(body: Value) -> FakeHub {
        let hits = Arc::new(AtomicUsize::new(0));
        let down = Arc::new(AtomicBool::new(false));
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get({
                let (hits, down) = (hits.clone(), down.clone());
                move || {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let reply = if down.load(Ordering::SeqCst) {
                        Err(axum::http::StatusCode::BAD_GATEWAY)
                    } else {
                        Ok(axum::Json(body.clone()))
                    };
                    async move { reply }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        FakeHub { url, hits, down }
    }

    fn client(url: &str) -> Arc<HubModels> {
        HubModels::new(
            url,
            "key".into(),
            HttpTimeouts {
                connect: Duration::from_secs(1),
                total: Duration::from_secs(1),
            },
        )
    }

    fn body() -> Value {
        json!({"data": [{"id": "gpt-6.1-sol", "owned_by": "openai", "created": 1}]})
    }

    #[tokio::test]
    async fn an_answer_is_cached_and_each_caller_gets_its_own_labels() {
        let hub = fake_hub(body()).await;
        let models = client(&hub.url);

        let labelled = models.models(&static_models()).await;
        let bare = models.models(&[]).await;

        assert_eq!(labelled[0].label, "GPT 6.1 Sol");
        assert_eq!(bare[0].label, "gpt-6.1-sol");
        assert_eq!(hub.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_refresh_serves_the_last_answer_and_waits_a_ttl_to_retry() {
        let hub = fake_hub(body()).await;
        let models = client(&hub.url);
        models.models(&[]).await;
        hub.down.store(true, Ordering::SeqCst);

        if let Some((at, _)) = models.cached.lock().await.as_mut() {
            *at = Instant::now()
                .checked_sub(CACHE_TTL + Duration::from_secs(1))
                .unwrap_or(*at);
        }
        let stale = models.models(&[]).await;
        let again = models.models(&[]).await;

        assert_eq!(ids(&stale), ["openai/gpt-6.1-sol"]);
        assert_eq!(ids(&again), ["openai/gpt-6.1-sol"]);
        assert_eq!(hub.hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_hub_that_never_answered_publishes_nothing() {
        let hub = fake_hub(body()).await;
        hub.down.store(true, Ordering::SeqCst);

        assert!(client(&hub.url).models(&static_models()).await.is_empty());
    }
}
