use std::time::Duration;

use moatless_api_schemas::decisions::{DecisionOption, DecisionUsage};
use serde_json::json;
use tokio::time::Instant;

use super::stub::{API_KEY, JevStub, Reply, UNREACHABLE, jev_config};
use crate::jev::{JevClient, retry_delay};
use crate::provider::{ProviderError, ProviderPick, Question};

fn options() -> Vec<DecisionOption> {
    vec![
        DecisionOption {
            name: "bug".to_string(),
            description: Some("Something is broken".to_string()),
        },
        DecisionOption {
            name: "feature".to_string(),
            description: None,
        },
    ]
}

async fn ask(base_url: &str, within: Duration) -> Result<ProviderPick, ProviderError> {
    let options = options();
    let question = Question {
        text: "Bug or feature?",
        options: &options,
        state: "The login button does nothing",
    };
    JevClient::new(&jev_config(base_url), timeouts())
        .pick(&question, Instant::now() + within)
        .await
}

#[tokio::test]
async fn an_answer_maps_and_the_request_carries_key_model_and_criteria_in_order() {
    let stub = JevStub::start(vec![Reply::answer(
        "bug",
        0.93,
        &[("bug", 0.95), ("feature", 0.05)],
    )])
    .await;

    let pick = ask(&stub.base_url, Duration::from_secs(5))
        .await
        .expect("answer");

    assert_eq!(pick.choice, "bug");
    assert_eq!(pick.confidence, Some(0.93));
    assert_eq!(pick.attempts, 1);
    assert_eq!(
        pick.usage,
        DecisionUsage {
            input_tokens: 812,
            output_tokens: 40
        }
    );
    let requests = stub.requests();
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(
        requests[0].body,
        json!({
            "model": "jev-1.13.0",
            "state": "The login button does nothing",
            "questions": {"q": {
                "type": "choice",
                "instructions": "Bug or feature?",
                "criteria": {"bug": "Something is broken", "feature": "feature"},
            }},
        })
    );
    let criteria = serde_json::to_string(&requests[0].body["questions"]["q"]["criteria"])
        .expect("criteria serialize");
    assert!(
        criteria.find("bug") < criteria.find("feature"),
        "{criteria}"
    );
}

#[tokio::test]
async fn a_rate_limit_or_overload_is_retried_once_then_answers() {
    for status in [429, 529] {
        let stub = JevStub::start(vec![
            Reply::status(status).retry_after(0),
            Reply::answer("feature", 0.6, &[]),
        ])
        .await;

        let pick = ask(&stub.base_url, Duration::from_secs(5))
            .await
            .expect("answer after a retry");

        assert_eq!(pick.choice, "feature", "status {status}");
        assert_eq!(pick.attempts, 2, "status {status}");
        assert_eq!(stub.hits(), 2, "status {status}");
    }
}

#[tokio::test]
async fn a_persistent_rate_limit_gives_up_after_two_retries() {
    let stub = JevStub::start(vec![Reply::status(429).retry_after(0)]).await;

    let error = ask(&stub.base_url, Duration::from_secs(5))
        .await
        .expect_err("overloaded");

    assert!(
        matches!(
            error,
            ProviderError::Overloaded {
                status: 429,
                attempts: 3
            }
        ),
        "{error:?}"
    );
    assert_eq!(stub.hits(), 3);
}

#[tokio::test]
async fn a_retry_after_past_the_deadline_is_not_waited_for() {
    let stub = JevStub::start(vec![Reply::status(529).retry_after(3)]).await;

    let started = Instant::now();
    let error = ask(&stub.base_url, Duration::from_secs(1))
        .await
        .expect_err("overloaded");

    assert!(
        matches!(error, ProviderError::Overloaded { attempts: 1, .. }),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn a_non_retryable_status_fails_on_the_first_attempt() {
    for (status, expected) in [
        (401, "auth"),
        (403, "auth"),
        (422, "rejected"),
        (500, "upstream"),
    ] {
        let stub = JevStub::start(vec![Reply::status(status)]).await;

        let error = ask(&stub.base_url, Duration::from_secs(5))
            .await
            .expect_err("provider error");

        let kind = match error {
            ProviderError::Auth { .. } => "auth",
            ProviderError::Rejected => "rejected",
            ProviderError::Upstream { status: 500 } => "upstream",
            other => panic!("status {status}: unexpected {other:?}"),
        };
        assert_eq!(kind, expected, "status {status}");
        assert_eq!(stub.hits(), 1, "status {status}");
    }
}

#[tokio::test]
async fn an_unreachable_provider_is_a_transport_error() {
    let error = ask(UNREACHABLE, Duration::from_secs(5))
        .await
        .expect_err("transport");

    assert!(matches!(error, ProviderError::Transport(_)), "{error:?}");
}

#[tokio::test]
async fn an_attempt_is_cut_off_at_the_deadline() {
    let stub = JevStub::start(vec![
        Reply::answer("bug", 0.9, &[]).delayed(Duration::from_secs(3)),
    ])
    .await;

    let started = Instant::now();
    let error = ask(&stub.base_url, Duration::from_millis(200))
        .await
        .expect_err("timed out");

    assert!(matches!(error, ProviderError::Transport(_)), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn an_undecodable_or_empty_answer_is_rejected() {
    let stub = JevStub::start(vec![Reply::raw(200, "not json")]).await;
    let error = ask(&stub.base_url, Duration::from_secs(5))
        .await
        .expect_err("decode");
    assert!(matches!(error, ProviderError::Decode(_)), "{error:?}");

    let stub = JevStub::start(vec![Reply::raw(200, r#"{"answers": {}}"#)]).await;
    let error = ask(&stub.base_url, Duration::from_secs(5))
        .await
        .expect_err("malformed");
    assert!(matches!(error, ProviderError::Malformed(_)), "{error:?}");
}

#[test]
fn retry_delay_backs_off_honours_retry_after_and_respects_the_deadline() {
    let plenty = Duration::from_secs(20);
    let ms = Duration::from_millis;
    let cases = [
        (1, None, plenty, Some(ms(250))),
        (2, None, plenty, Some(ms(1000))),
        (3, None, plenty, None),
        (1, Some(ms(2000)), plenty, Some(ms(2000))),
        (1, Some(ms(30_000)), plenty, Some(ms(4000))),
        (1, None, ms(250), None),
        (1, Some(ms(3000)), ms(1000), None),
        (0, None, plenty, None),
    ];
    for (attempts, retry_after, remaining, expected) in cases {
        assert_eq!(
            retry_delay(attempts, retry_after, remaining),
            expected,
            "attempts {attempts}, retry_after {retry_after:?}, remaining {remaining:?}"
        );
    }
}

#[test]
fn the_client_debug_never_shows_the_key() {
    let rendered = format!("{:?}", JevClient::new(&jev_config(UNREACHABLE), timeouts()));

    assert!(!rendered.contains(API_KEY), "{rendered}");
    assert!(rendered.contains("[REDACTED]"), "{rendered}");
}

#[tokio::test]
async fn a_confidence_outside_zero_to_one_is_dropped() {
    for out_of_range in [1.5, -0.1] {
        let stub = JevStub::start(vec![Reply::answer("bug", out_of_range, &[])]).await;

        let pick = ask(&stub.base_url, Duration::from_secs(5))
            .await
            .expect("answer");

        assert_eq!(pick.confidence, None, "{out_of_range}");
    }
}

fn timeouts() -> moatless_types::http::HttpTimeouts {
    moatless_core::config::HttpConfig::default().decision
}
