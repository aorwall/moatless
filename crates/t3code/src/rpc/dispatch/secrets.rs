//! `secrets.answerRequest`: a person answers the secret card a Task's agent
//! showed in its thread.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::ports::SecretRequestAnswerError;
use crate::rpc::envelope::{RequestId, failure_exit, success_exit, typed_failure_exit};
use crate::rpc::errors::authorization_error;
use crate::rpc::table::{MethodTable, Request};
use moatless_tasks::secret_request::service::Answer;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::error;

/// The prefix `projection::v2` gives an activity's turn item id.
const ACTIVITY_ITEM_PREFIX: &str = "item:activity:";

pub(super) fn secret_methods() -> MethodTable {
    MethodTable::new().session(
        "secrets.answerRequest",
        Scope::OrchestrationOperate,
        secrets_answer_request,
    )
}

/// The contract's `SecretRequestAnswerInput`. `answer` stays a [`Value`]: a
/// typed field's decode error quotes what it refused, which here is the secret.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SecretRequestAnswerInput {
    thread_id: String,
    turn_item_id: String,
    answer: Value,
}

fn answer_of(answer: &Value) -> Option<Answer> {
    match answer.get("type").and_then(Value::as_str)? {
        "save" => answer
            .get("secret")
            .and_then(Value::as_str)
            .map(|secret| Answer::Save(secret.to_string())),
        "decline" => Some(Answer::Decline),
        _ => None,
    }
}

pub(super) async fn secrets_answer_request(
    request: Request<SecretRequestAnswerInput>,
) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(answer) = answer_of(&payload.answer) else {
        return vec![failure_exit(
            &id,
            "secrets.answerRequest: unreadable answer",
        )];
    };
    let Some(activity_id) = payload
        .turn_item_id
        .strip_prefix(ACTIVITY_ITEM_PREFIX)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
    else {
        return vec![request_failure(&id, "not_found")];
    };
    let answered = state
        .secret_requests
        .answer(
            &viewer_scope(&identity),
            &identity.login,
            &payload.thread_id,
            activity_id,
            answer,
        )
        .await;
    match answered {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(SecretRequestAnswerError::NotFound) => vec![request_failure(&id, "not_found")],
        Err(SecretRequestAnswerError::AlreadyAnswered) => {
            vec![request_failure(&id, "already_answered")]
        }
        Err(SecretRequestAnswerError::Refused(message)) => {
            vec![typed_failure_exit(&id, authorization_error(scope, message))]
        }
        Err(SecretRequestAnswerError::Failed(cause)) => {
            error!(%cause, user_id = %identity.user_id, "Failed to answer a secret request");
            vec![request_failure(&id, "store_failed")]
        }
    }
}

/// The contract's `SecretRequestError`, whose `reason` the card turns into
/// its own copy.
fn request_failure(id: &RequestId, reason: &str) -> Value {
    typed_failure_exit(
        id,
        json!({ "_tag": "SecretRequestError", "reason": reason }),
    )
}
