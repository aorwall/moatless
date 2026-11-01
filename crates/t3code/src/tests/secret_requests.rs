//! Answering a secret request, against a mocked answerer.

use super::fixtures::*;
use super::*;
use crate::ports::{MockSecretRequestAnswerer, SecretRequestAnswerError};
use moatless_tasks::secret_request::service::Answer;

const ACTIVITY: &str = "6f1c2a52-6c9d-4f5e-9a63-3b0b9f0b6c11";

async fn answer(answerer: MockSecretRequestAnswerer, turn_item_id: &str, answer: Value) -> Value {
    let state = UiRpcState {
        secret_requests: Arc::new(answerer),
        ..state()
    };
    let frame = request(
        "1",
        "secrets.answerRequest",
        json!({ "threadId": "task-1", "turnItemId": turn_item_id, "answer": answer }),
    );
    let mut session = session();
    let output = session.handle_json(&frame, &state, &viewer()).await;
    output.frames[0]["exit"].clone()
}

#[tokio::test]
async fn a_saved_answer_reaches_the_answerer_with_its_value_and_activity() {
    let mut answerer = MockSecretRequestAnswerer::new();
    answerer
        .expect_answer()
        .withf(|_, _, task_id, activity_id, answer| {
            task_id == "task-1"
                && activity_id.to_string() == ACTIVITY
                && matches!(answer, Answer::Save(value) if value == "s3cret")
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));

    let exit = answer(
        answerer,
        &format!("item:activity:{ACTIVITY}"),
        json!({ "type": "save", "secret": "s3cret" }),
    )
    .await;

    assert_eq!(exit["_tag"], "Success", "{exit}");
}

/// The card turns each `reason` into its own copy; a refusal's message is
/// shown as sent.
#[tokio::test]
async fn each_failure_is_the_error_the_card_reads() {
    for (failure, tag, field, expected) in [
        (
            SecretRequestAnswerError::NotFound,
            "SecretRequestError",
            "reason",
            "not_found",
        ),
        (
            SecretRequestAnswerError::AlreadyAnswered,
            "SecretRequestError",
            "reason",
            "already_answered",
        ),
        (
            SecretRequestAnswerError::Failed("db down".to_string()),
            "SecretRequestError",
            "reason",
            "store_failed",
        ),
        (
            SecretRequestAnswerError::Refused(
                "Only an admin can save a global secret.".to_string(),
            ),
            "EnvironmentAuthorizationError",
            "message",
            "Only an admin can save a global secret.",
        ),
    ] {
        let mut answerer = MockSecretRequestAnswerer::new();
        let mut failure = Some(failure);
        answerer
            .expect_answer()
            .returning(move |_, _, _, _, _| Err(failure.take().expect("called once")));

        let exit = answer(
            answerer,
            &format!("item:activity:{ACTIVITY}"),
            json!({ "type": "decline" }),
        )
        .await;

        let error = &exit["cause"][0]["error"];
        assert_eq!(error["_tag"], tag, "{exit}");
        assert_eq!(error[field], expected, "{exit}");
    }
}

#[tokio::test]
async fn an_item_that_is_not_an_activity_is_not_found_without_asking() {
    let mut answerer = MockSecretRequestAnswerer::new();
    answerer.expect_answer().never();

    let exit = answer(answerer, "item:message:abc", json!({ "type": "decline" })).await;

    assert_eq!(exit["cause"][0]["error"]["reason"], "not_found", "{exit}");
}

/// A typed decode would quote the value it refused, which here is the secret.
#[tokio::test]
async fn a_malformed_answer_is_refused_without_quoting_it() {
    let mut answerer = MockSecretRequestAnswerer::new();
    answerer.expect_answer().never();

    let exit = answer(
        answerer,
        &format!("item:activity:{ACTIVITY}"),
        json!({ "type": "save", "secret": 424_242 }),
    )
    .await;

    assert!(!exit.to_string().contains("424242"), "{exit}");
}
