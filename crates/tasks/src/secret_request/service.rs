//! Asking for a secret and answering the request.

use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_core::secret_keys::{
    ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_BASE_URL, CLAUDE_CODE_OAUTH_TOKEN,
    CODEX_AUTH_JSON, GITHUB_TOKEN, GITLAB_TOKEN, OPENAI_API_KEY, OPENAI_BASE_URL,
    is_forgejo_credential_key, is_reserved_key, is_valid_secret_key,
};
use moatless_identity::secret::error::SecretError;
use moatless_identity::secret::schemas::PutSecretRequest;
use moatless_identity::secret::service::SecretService;
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_types::activity::{SecretRequestAnswered, SecretRequestOutcome, SecretRequested};
use moatless_types::credential_key::{GH_TOKEN, GIT_TOKEN};
use moatless_types::secret::SecretKind;
use moatless_types::task::ExternalOrigin;
use uuid::Uuid;

use super::dao;
use super::error::SecretRequestError;
use crate::message::send::MessageSendService;
use crate::task::operations::TaskCommands;

const MAX_LABEL_CHARS: usize = 120;
const MAX_REASON_CHARS: usize = 1000;

/// What an agent asks for. The value is never part of it.
#[derive(Debug, Clone)]
pub struct NewSecretRequest {
    pub key: String,
    pub scope: Scope,
    pub kind: SecretKind,
    /// The card's heading; the key when absent.
    pub label: Option<String>,
    pub reason: String,
}

/// A person's answer to a request.
pub enum Answer {
    Save(String),
    Decline,
}

impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Save(_) => f.write_str("Save([REDACTED])"),
            Self::Decline => f.write_str("Decline"),
        }
    }
}

/// Secret requests over the Task, secret and message stores they write.
pub struct SecretRequests {
    db: DbPool,
    tasks: Arc<TaskCommands>,
    secrets: Arc<SecretService>,
    messages: Arc<MessageSendService>,
}

impl SecretRequests {
    pub fn new(
        db: DbPool,
        tasks: Arc<TaskCommands>,
        secrets: Arc<SecretService>,
        messages: Arc<MessageSendService>,
    ) -> Self {
        Self {
            db,
            tasks,
            secrets,
            messages,
        }
    }

    /// Ask for a secret on `task_id` as `ctx`, who must be able to change the
    /// Task. The card shows in the thread until someone answers it.
    pub async fn request(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
        req: NewSecretRequest,
    ) -> Result<SecretRequested, SecretRequestError> {
        self.writable_task(ctx, task_id).await?;
        let asked = asked(req, ctx).map_err(SecretRequestError::BadRequest)?;
        dao::create(&self.db, task_id, &asked).await?;
        Ok(asked)
    }

    /// Answer the request whose activity is `activity_id`, as `viewer`.
    ///
    /// A saved value is written before the answer is recorded, so a failed
    /// write leaves the request open to try again rather than marked saved.
    pub async fn answer(
        &self,
        viewer: &ScopeContext,
        login: &str,
        task_id: &str,
        activity_id: Uuid,
        answer: Answer,
    ) -> Result<(), SecretRequestError> {
        self.writable_task(viewer, task_id).await?;
        let (asked, answered) = dao::find(&self.db, task_id, activity_id)
            .await?
            .ok_or_else(|| SecretRequestError::NotFound(activity_id.to_string()))?;
        if answered {
            return Err(SecretRequestError::AlreadyAnswered);
        }
        may_answer(viewer, &asked).map_err(SecretRequestError::Forbidden)?;
        let outcome = match answer {
            Answer::Save(value) => {
                self.save(viewer, &asked, value).await?;
                SecretRequestOutcome::Saved
            }
            Answer::Decline => SecretRequestOutcome::Declined,
        };
        let answered = SecretRequestAnswered {
            request_id: asked.request_id.clone(),
            outcome,
            answered_by: viewer.user_id.clone(),
            answered_by_login: login.to_string(),
        };
        if !dao::record_answer(&self.db, task_id, &answered).await? {
            return Err(SecretRequestError::AlreadyAnswered);
        }
        self.notify(task_id, &asked, &answered).await;
        Ok(())
    }

    async fn writable_task(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
    ) -> Result<(), SecretRequestError> {
        let task = self
            .tasks
            .find_by_id(task_id, ctx)
            .await?
            .ok_or_else(|| SecretRequestError::TaskNotFound(task_id.to_string()))?;
        if !ctx.can_write(&task.scoped_resource(), Some(task.visibility)) {
            return Err(SecretRequestError::Forbidden(
                "Only someone who can change this thread can ask for or answer its secrets."
                    .to_string(),
            ));
        }
        Ok(())
    }

    async fn save(
        &self,
        viewer: &ScopeContext,
        asked: &SecretRequested,
        value: String,
    ) -> Result<(), SecretRequestError> {
        if value.trim().is_empty() {
            return Err(SecretRequestError::BadRequest(
                "The secret is empty.".to_string(),
            ));
        }
        let req = PutSecretRequest {
            scope: Some(asked.scope),
            key: asked.key.clone(),
            kind: asked.kind,
            provider: None,
            value: Some(value),
            external_ref: None,
        };
        match self.secrets.put_secret(viewer, &viewer.user_id, req).await {
            Ok(_) => Ok(()),
            Err(SecretError::Forbidden(message)) => Err(SecretRequestError::Forbidden(message)),
            Err(SecretError::BadRequest(message)) => Err(SecretRequestError::BadRequest(message)),
            Err(other) => Err(SecretRequestError::Store(other)),
        }
    }

    /// Wake the Task with what happened. The answer stands when this fails:
    /// the card already shows it.
    async fn notify(
        &self,
        task_id: &str,
        asked: &SecretRequested,
        answered: &SecretRequestAnswered,
    ) {
        let text = notice_text(asked, answered);
        let event_id = format!("secret_request:{}", asked.request_id);
        let sent = self
            .messages
            .send_message_flow(
                task_id,
                &text,
                None,
                None,
                None,
                &[],
                &[],
                true,
                ExternalOrigin {
                    event_id: Some(&event_id),
                    author: Some(&answered.answered_by_login),
                    ..ExternalOrigin::none()
                },
                false,
                Some(&answered.answered_by),
                &[],
                None,
                true,
                None,
                None,
            )
            .await;
        if let Err(e) = sent {
            tracing::warn!(
                error = %e,
                task_id = %task_id,
                request_id = %asked.request_id,
                "Failed to tell the Task its secret request was answered"
            );
        }
    }
}

/// The request `req` describes, made by `ctx`, or why it may not be made.
fn asked(req: NewSecretRequest, ctx: &ScopeContext) -> Result<SecretRequested, String> {
    let key = req.key.trim().to_string();
    if !is_valid_secret_key(&key) {
        return Err(format!("secret key must match ^[A-Z_][A-Z0-9_]*$: {key}"));
    }
    if !is_requestable_key(&key) {
        return Err(format!(
            "{key} is a platform credential; set it with `moat secret set` or the provider's own sign-in"
        ));
    }
    if !matches!(req.kind, SecretKind::Env | SecretKind::RuntimeFile) {
        return Err(format!(
            "a request may ask only for an env or runtime_file secret, not {}",
            req.kind
        ));
    }
    let label = req
        .label
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| key.clone());
    if label.chars().count() > MAX_LABEL_CHARS {
        return Err(format!("label is longer than {MAX_LABEL_CHARS} characters"));
    }
    let reason = req.reason.trim().to_string();
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(format!(
            "reason is longer than {MAX_REASON_CHARS} characters"
        ));
    }
    Ok(SecretRequested {
        request_id: Uuid::new_v4().to_string(),
        key,
        scope: req.scope,
        kind: req.kind,
        label,
        reason,
        requested_by: ctx.user_id.clone(),
    })
}

/// Whether a person may be asked for `key`. Platform credentials are refused:
/// a misled agent that asks someone to replace `GITHUB_TOKEN` or a model key
/// swaps in an account it controls.
fn is_requestable_key(key: &str) -> bool {
    const PLATFORM_CREDENTIALS: &[&str] = &[
        GITHUB_TOKEN,
        GITLAB_TOKEN,
        GH_TOKEN,
        GIT_TOKEN,
        ANTHROPIC_API_KEY,
        ANTHROPIC_AUTH_TOKEN,
        ANTHROPIC_BASE_URL,
        CLAUDE_CODE_OAUTH_TOKEN,
        CODEX_AUTH_JSON,
        OPENAI_API_KEY,
        OPENAI_BASE_URL,
    ];
    !is_reserved_key(key) && !is_forgejo_credential_key(key) && !PLATFORM_CREDENTIALS.contains(&key)
}

/// Why `viewer` may not answer `asked`, if they may not.
///
/// A `user` request is the requester's alone: the Sandbox resolves user
/// secrets for the user its token names, so a value saved under anyone else's
/// user scope would never reach it.
fn may_answer(viewer: &ScopeContext, asked: &SecretRequested) -> Result<(), String> {
    match asked.scope {
        Scope::User if viewer.user_id != asked.requested_by => Err(
            "This request saves to the requester's own secrets, so only they can answer it."
                .to_string(),
        ),
        Scope::Global if !viewer.is_admin => {
            Err("Only an admin can save a global secret.".to_string())
        }
        Scope::User | Scope::Global => Ok(()),
    }
}

fn notice_text(asked: &SecretRequested, answered: &SecretRequestAnswered) -> String {
    let key = &asked.key;
    let login = &answered.answered_by_login;
    match answered.outcome {
        SecretRequestOutcome::Saved => format!(
            "{login} saved the secret `{key}` to the {scope} secrets ({kind}). \
             It is in the agent's environment from the next turn, not this one; \
             the value was never shown in the conversation.",
            scope = asked.scope,
            kind = asked.kind,
        ),
        SecretRequestOutcome::Declined => {
            format!("{login} declined the request for the secret `{key}`.")
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
