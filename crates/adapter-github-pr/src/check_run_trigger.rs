//! Tasks a trusted check run opens, under a `create_on: check_run` loop.
//!
//! A completed check run matching the loop's `check_run` names, App ids and
//! conclusions triggers it. Its `output.text` may carry a [`MARKER_PREFIX`]
//! marker naming the PR, head, skills, agent, model and a message. The durable delivery
//! fetches the PR, finds its Workspace, and opens the Task, or adds new skills
//! to the Task an earlier trigger opened. A review gate is the motivating App.

use tracing::{debug, info, warn};

use moatless_adapter_model::binding::BindScope;
use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_adapter_model::error::AdapterError;
use moatless_core::agent_catalog::accepts_model;
use moatless_ports::task::TaskOpenError;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{AdapterConnection, AdapterKind, Audience, InboundMessage};
use moatless_types::agent::AgentProfile;
use moatless_types::loops::SubscriptionMatch;
use moatless_types::store::StoreError;
use moatless_types::task::Visibility;

use super::api::PrInfo;
use super::config::{CHECK_RUN_TRIGGER, CreateOn, GithubPrRuleConfig};
use super::router::GithubPrInboundRouter;
use super::routing::{ADAPTER, ADAPTER_LABEL, SUBJECT_KIND};
use super::subject::GithubPrSubject;
use super::text::{CheckRunTriggerText, PrOpenedText, check_run_trigger_text, pr_opened_text};
use super::webhook::{IgnoredWebhook, NormalizeContext, PrimaryWebhook};

/// Opens the JSON marker in a check run's `output.text`; ` -->` closes it.
pub(super) const MARKER_PREFIX: &str = "<!-- moatless-request:v1 ";

/// Binding `metadata` key holding every skill triggers asked of the Task, so
/// a repeat trigger can tell which of its skills are new.
const REQUESTED_SKILLS_KEY: &str = "requested_skills";

/// Binding `metadata` key holding the head the last delivered trigger named,
/// so a repeat trigger can tell a new head from a redelivery.
const REQUESTED_HEAD_KEY: &str = "requested_head";

/// One rule of the triggering App that matched the PR, and its skills.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct MatchedRule {
    pub(super) id: String,
    #[serde(default)]
    pub(super) skills: Vec<String>,
}

/// The optional marker. Every field is optional.
#[derive(Default, serde::Deserialize)]
struct Marker {
    /// The PR to act on. GitHub leaves `pull_requests` empty for a fork's head
    /// and lists every PR on a shared head, so this wins.
    #[serde(default)]
    pr: Option<u64>,
    /// When present, must equal `check_run.head_sha`.
    #[serde(default)]
    head: Option<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    matched: Vec<MatchedRule>,
    #[serde(default)]
    message: Option<String>,
    /// The Task's agent profile, such as `codex` ([`trigger_agent`]).
    #[serde(default)]
    agent: Option<String>,
    /// The Task's model, used when its agent accepts it ([`trigger_agent`]).
    #[serde(default)]
    model: Option<String>,
}

/// Facts persisted for one durable check-run trigger.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Execution {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) pr_number: u64,
    pub(super) head_sha: String,
    pub(super) check_name: String,
    pub(super) conclusion: String,
    pub(super) output_title: String,
    pub(super) output_summary: String,
    pub(super) skills: Vec<String>,
    pub(super) matched: Vec<MatchedRule>,
    pub(super) message: Option<String>,
    /// Defaulted: an execution stored before the marker carried an agent loads.
    #[serde(default)]
    pub(super) agent: Option<String>,
    /// Defaulted: an execution stored before the marker carried a model loads.
    #[serde(default)]
    pub(super) model: Option<String>,
    /// From the webhook's `repository`, for registering an unknown repository.
    pub(super) default_branch: Option<String>,
    pub(super) description: Option<String>,
}

/// The Workspace a triggered Task on one repository runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerWorkspaceRequest {
    pub owner: String,
    pub repo: String,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    /// `provision.workspace_template`.
    pub template: Option<String>,
    /// The loop's own Workspace, used when it holds the repository.
    pub loop_workspace_id: String,
    /// The loop's creator, an administrator: whoever registers what is made.
    pub acting_user_id: String,
}

/// Where a triggered Task runs, or why no Task is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerWorkspace {
    Found(String),
    /// Logged and dropped: no redelivery changes a deleted repository or a
    /// missing template.
    Skipped(String),
}

/// Finds or creates the Workspace for a triggered Task's repository.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait TriggerWorkspaces: Send + Sync {
    async fn workspace_for(
        &self,
        request: &TriggerWorkspaceRequest,
    ) -> Result<TriggerWorkspace, StoreError>;
}

/// The trigger in a `check_run` payload, or `None` when the check run does not
/// match this loop's trigger and normalizes as an ordinary check run.
///
/// Runs before the event filter and the failure-conclusion filter: a trigger
/// may be neutral, and is filtered by [`CHECK_RUN_TRIGGER`], not `check_run`.
pub(super) fn normalize(
    event_type: &str,
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
    config: &GithubPrRuleConfig,
) -> Option<PrimaryWebhook> {
    if event_type != "check_run" || config.create_on != CreateOn::CheckRun {
        return None;
    }
    let check_run = payload.get("check_run")?;
    let field = |pointer: &str| {
        check_run
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
    };
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("completed")
        || !config.check_run.matches(
            field("/name").unwrap_or_default(),
            check_run
                .pointer("/app/id")
                .and_then(serde_json::Value::as_u64),
            field("/conclusion").unwrap_or_default(),
        )
    {
        return None;
    }
    if config.event_filtered_out(CHECK_RUN_TRIGGER) {
        return Some(PrimaryWebhook::Ignore(IgnoredWebhook::EventFiltered));
    }
    Some(match validate(payload, check_run, context) {
        Ok(execution) => PrimaryWebhook::CheckRunTrigger(Box::new(execution)),
        Err(reason) => {
            info!(
                loop_id = %context.rule.id,
                owner = context.owner,
                repo = context.repo,
                reason,
                "Ignoring a check-run trigger"
            );
            PrimaryWebhook::Ignore(IgnoredWebhook::InvalidCheckRunTrigger)
        }
    })
}

fn validate(
    payload: &serde_json::Value,
    check_run: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<Execution, &'static str> {
    let field = |pointer: &str| {
        check_run
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let marker = match field("/output/text").split_once(MARKER_PREFIX) {
        None => Marker::default(),
        Some((_, rest)) => rest
            .split_once(" -->")
            .and_then(|(json, _)| serde_json::from_str::<Marker>(json).ok())
            .ok_or("the marker in output.text is not readable JSON")?,
    };
    if !marker.skills.iter().all(|skill| is_skill_name(skill)) {
        return Err("the marker names a skill that is not a skill directory name");
    }
    let head_sha = field("/head_sha");
    if head_sha.is_empty() {
        return Err("the check run has no head_sha");
    }
    if marker.head.as_ref().is_some_and(|head| *head != head_sha) {
        return Err("the marker's head is not the check run's head");
    }
    let pr_number = marker
        .pr
        .or_else(|| {
            check_run
                .pointer("/pull_requests/0/number")
                .and_then(serde_json::Value::as_u64)
        })
        .ok_or("neither the marker nor the check run names a pull request")?;
    let repository_field = |name: &str| {
        payload
            .pointer(&format!("/repository/{name}"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    Ok(Execution {
        owner: context.owner.to_string(),
        repo: context.repo.to_string(),
        pr_number,
        head_sha,
        check_name: field("/name"),
        conclusion: field("/conclusion"),
        output_title: field("/output/title"),
        output_summary: field("/output/summary"),
        skills: marker.skills,
        matched: marker.matched,
        message: marker.message.filter(|m| !m.trim().is_empty()),
        agent: marker.agent.filter(|a| !a.trim().is_empty()),
        model: marker.model.filter(|m| !m.trim().is_empty()),
        default_branch: repository_field("default_branch"),
        description: repository_field("description"),
    })
}

/// A skill directory name: no path separator, no leading dot, nothing a
/// sandbox would resolve outside its skills directory.
fn is_skill_name(skill: &str) -> bool {
    !skill.is_empty()
        && !skill.starts_with('.')
        && skill
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// The agent profile and model a triggered Task opens with: the marker's agent
/// when known, and its model when that agent accepts it ([`accepts_model`]),
/// else the loop's model under the loop's agent or the agent's default. A bad
/// value is logged and passed over rather than refused, so a mistyped policy
/// still gets its review.
pub(super) fn trigger_agent(
    rule: &SubscriptionMatch,
    request: &Execution,
) -> (AgentProfile, Option<String>) {
    let agent_profile = match request.agent.as_deref() {
        Some(agent) => AgentProfile::parse(agent).unwrap_or_else(|| {
            warn!(loop_id = %rule.id, agent, "Check-run trigger's agent is unknown; using the loop's agent");
            rule.agent_profile
        }),
        None => rule.agent_profile,
    };
    let fallback = if agent_profile == rule.agent_profile {
        rule.model.clone()
    } else {
        None
    };
    let model = match request.model.as_deref() {
        Some(model) if accepts_model(agent_profile, model) => Some(model.to_string()),
        Some(model) => {
            warn!(
                loop_id = %rule.id,
                model,
                agent_profile = %agent_profile,
                "Check-run trigger's model is not one its agent accepts; using the default"
            );
            fallback
        }
        None => fallback,
    };
    (agent_profile, model)
}

/// `requested` minus everything in `held`, in request order, without repeats.
fn new_skills(requested: &[String], held: &[String]) -> Vec<String> {
    let mut fresh: Vec<String> = Vec::new();
    for skill in requested {
        if !held.contains(skill) && !fresh.contains(skill) {
            fresh.push(skill.clone());
        }
    }
    fresh
}

/// The open triggered Task's Binding under `loop_id`. A closed Task's Binding
/// is passed over, so a later trigger opens a new Task by taking over the
/// closed one's creation claim rather than being dropped.
fn open_trigger_binding(bindings: Vec<TaskBinding>, loop_id: &str) -> Option<TaskBinding> {
    bindings.into_iter().find(|b| {
        b.active
            && b.origin == super::routing::ORIGIN_ADAPTER_EVENT
            && b.loop_id.as_deref() == Some(loop_id)
    })
}

fn requested_skills(binding: &TaskBinding) -> Vec<String> {
    binding
        .metadata
        .get(REQUESTED_SKILLS_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn requested_head(binding: &TaskBinding) -> Option<&str> {
    binding
        .metadata
        .get(REQUESTED_HEAD_KEY)
        .and_then(serde_json::Value::as_str)
}

/// The skills a repeat trigger adds to the open Task and whether it names a
/// head the Task was not asked about, or `None` when it adds neither. A
/// Binding stored before heads were recorded counts every head as new.
fn repeat_delta(
    binding: &TaskBinding,
    loop_skills: &[String],
    request: &Execution,
) -> Option<(Vec<String>, bool)> {
    let mut held = requested_skills(binding);
    held.extend(loop_skills.iter().cloned());
    let fresh = new_skills(&request.skills, &held);
    let new_head = requested_head(binding) != Some(request.head_sha.as_str());
    (new_head || !fresh.is_empty()).then_some((fresh, new_head))
}

/// The ids of the matched rules that asked for any of `skills`, or `None`
/// when no rule names them.
fn rules_asking_for(matched: &[MatchedRule], skills: &[String]) -> Option<String> {
    let ids: Vec<&str> = matched
        .iter()
        .filter(|rule| rule.skills.iter().any(|s| skills.contains(s)))
        .map(|rule| rule.id.as_str())
        .collect();
    (!ids.is_empty()).then(|| ids.join(", "))
}

/// The turn a repeat trigger sends the open Task: the `fresh` skills it
/// added, and on a `new_head` the check's output, which says why the trigger
/// asked again — such as a raised tier — where the push event cannot.
fn repeat_text(request: &Execution, fresh: &[String], new_head: bool) -> String {
    let sha7: String = request.head_sha.chars().take(7).collect();
    let mut text = if fresh.is_empty() {
        format!(
            "The {} check requested a review of {sha7}.",
            request.check_name
        )
    } else {
        let rules = rules_asking_for(&request.matched, fresh)
            .map_or_default(|ids| format!(" (rule {ids})"));
        format!(
            "The {} check added {} for {sha7}{rules}.",
            request.check_name,
            fresh.join(", "),
        )
    };
    if new_head {
        text.push_str(&check_run_trigger_text(&CheckRunTriggerText {
            name: &request.check_name,
            conclusion: &request.conclusion,
            output_title: &request.output_title,
            output_summary: &request.output_summary,
        }));
    }
    if let Some(message) = &request.message {
        text.push_str("\n\n");
        text.push_str(message);
    }
    text
}

/// The first turn: the PR opening text, the triggering check, and the
/// marker's message.
fn first_turn_text(opened: &str, request: &Execution) -> String {
    let mut text = opened.to_string();
    text.push_str(&check_run_trigger_text(&CheckRunTriggerText {
        name: &request.check_name,
        conclusion: &request.conclusion,
        output_title: &request.output_title,
        output_summary: &request.output_summary,
    }));
    if let Some(message) = &request.message {
        text.push('\n');
        text.push_str(message);
        text.push('\n');
    }
    text
}

impl GithubPrInboundRouter {
    /// Open the Task a trusted check run triggered, or send the open Task a
    /// repeat trigger that names a new head or a skill it does not hold yet.
    pub(super) async fn run_check_run_trigger(
        &self,
        connection: &AdapterConnection,
        trigger: Option<&SubscriptionMatch>,
        request: &Execution,
    ) -> Result<(), AdapterError> {
        let subject = GithubPrSubject::new(&request.owner, &request.repo, request.pr_number);
        let subject_key = subject.key();
        // Re-checked: the loop may have been stopped or edited since admission.
        let Some(rule) = trigger.filter(|rule| {
            rule.active && GithubPrRuleConfig::from_rule(rule).create_on == CreateOn::CheckRun
        }) else {
            info!(subject_key = %subject_key, "Check-run trigger dropped: its loop no longer takes check-run triggers");
            return Ok(());
        };

        let bindings = self
            .task_bindings
            .for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
            .await?;
        if let Some(binding) = open_trigger_binding(bindings, &rule.id) {
            return self.deliver_repeat_trigger(rule, &binding, request).await;
        }
        let Some(pr) = self.current_pr(rule, request).await? else {
            return Ok(());
        };
        let author = pr.user.as_ref().map_or("", |user| user.login.as_str());
        let opened = pr_opened_text(&PrOpenedText {
            pr_number: request.pr_number,
            title: &pr.title,
            sender: author,
            head_ref: &pr.head.ref_name,
            base_ref: &pr.base.ref_name,
            html_url: pr.html_url.as_deref().unwrap_or_default(),
            owner: &request.owner,
            repo: &request.repo,
        });
        let text = first_turn_text(&opened, request);
        let message = self
            .trigger_message(rule, request, author, text, None)
            .await;
        self.open_trigger_task(connection, rule, request, message)
            .await
    }

    /// The pull request `request` names, or `None` (logged) when it is closed,
    /// a draft, or its head has moved past the requested one.
    async fn current_pr(
        &self,
        rule: &SubscriptionMatch,
        request: &Execution,
    ) -> Result<Option<PrInfo>, AdapterError> {
        let subject_key =
            GithubPrSubject::new(&request.owner, &request.repo, request.pr_number).key();
        let Some(adapter) = self.adapter.as_ref() else {
            warn!(subject_key = %subject_key, "Check-run trigger dropped: GitHub is not configured");
            return Ok(None);
        };
        let api = super::auth::resolve_github_api(
            adapter,
            rule.effective_user_id(),
            None,
            &request.owner,
            &request.repo,
        )
        .await?;
        let pr = api
            .get_pr(&request.owner, &request.repo, request.pr_number)
            .await?;
        if pr.state != "open" || pr.draft.unwrap_or(false) || pr.head.sha != request.head_sha {
            info!(
                subject_key = %subject_key,
                state = %pr.state,
                draft = pr.draft.unwrap_or(false),
                head = %pr.head.sha,
                requested_head = %request.head_sha,
                "Check-run trigger dropped: the pull request is closed, a draft, or has moved on"
            );
            return Ok(None);
        }
        Ok(Some(pr))
    }

    /// Resolve the Workspace, open the Task with the loop's and the trigger's
    /// skills, bind it, and deliver `message` as its first turn.
    async fn open_trigger_task(
        &self,
        connection: &AdapterConnection,
        rule: &SubscriptionMatch,
        request: &Execution,
        message: InboundMessage,
    ) -> Result<(), AdapterError> {
        let subject_key =
            GithubPrSubject::new(&request.owner, &request.repo, request.pr_number).key();
        let config = GithubPrRuleConfig::from_rule(rule);
        let workspace = self
            .trigger_workspaces
            .workspace_for(&TriggerWorkspaceRequest {
                owner: request.owner.clone(),
                repo: request.repo.clone(),
                default_branch: request.default_branch.clone(),
                description: request.description.clone(),
                template: config.workspace_template.clone(),
                loop_workspace_id: rule.workspace_id.clone(),
                acting_user_id: rule.created_by.clone(),
            })
            .await
            .map_err(|e| AdapterError::task_handoff("find the triggered Task's workspace", e))?;
        let workspace_id = match workspace {
            TriggerWorkspace::Found(id) => id,
            TriggerWorkspace::Skipped(reason) => {
                warn!(subject_key = %subject_key, loop_id = %rule.id, reason, "Check-run trigger dropped: no workspace");
                return Ok(());
            }
        };

        let mut context = moatless_adapter_model::loops::subscription_context(rule);
        context.workspace_id = workspace_id;
        (context.agent_profile, context.model) = trigger_agent(rule, request);
        context.skills = [
            context.skills.clone(),
            new_skills(&request.skills, &context.skills),
        ]
        .concat();
        let reply_instructions = match self.adapter.as_ref() {
            Some(adapter) => adapter.reply_instructions().await,
            None => None,
        };
        self.routing
            .claim_task_creation(
                connection,
                rule,
                SUBJECT_KIND,
                &subject_key,
                ADAPTER_LABEL,
                message.external_event_id.as_deref(),
            )
            .await?;
        let opened = moatless_loops::loops::firing::inbound::create_task_from_loop(
            self.task_creation.as_ref(),
            &context,
            moatless_loops::loops::firing::inbound::CreateTaskParams {
                name: rule.task_name.clone(),
                description: moatless_loops::bindings::outbound::truncate_text(&message.text, 200),
                visibility: Visibility::Public,
                origin_prelude: Some(super::routing::PR_ORIGIN_CONTEXT),
                reply_instructions,
                related_to: None,
                event_metadata: serde_json::json!({
                    "adapter": ADAPTER_LABEL,
                    "source_matcher": rule.source_matcher,
                    "check_run_trigger": {
                        "check": request.check_name,
                        "head": request.head_sha,
                        "matched": request.matched,
                    },
                }),
                requested_by: None,
            },
        )
        .await;
        let task_id = match opened {
            Ok(opened) => opened.task_id,
            Err(TaskOpenError::Refused(reason)) => {
                self.routing
                    .release_task_creation(&rule.id, SUBJECT_KIND, &subject_key)
                    .await?;
                warn!(subject_key = %subject_key, loop_id = %rule.id, error = %reason, "Triggered Task failed preflight");
                return Ok(());
            }
            Err(other) => {
                self.routing
                    .release_task_creation(&rule.id, SUBJECT_KIND, &subject_key)
                    .await?;
                return Err(AdapterError::task_handoff(
                    "open a task from a check-run trigger",
                    other,
                ));
            }
        };

        let binding = self
            .task_bindings
            .bind(
                connection,
                Some(rule),
                &task_id,
                ADAPTER,
                SUBJECT_KIND,
                &subject_key,
                GithubPrSubject::from_thread_properties(&message.thread_properties)?
                    .destination_config(None),
                super::routing::ORIGIN_ADAPTER_EVENT,
                None,
                &BindScope::default(),
            )
            .await?;
        self.routing
            .record_created_task(&rule.id, SUBJECT_KIND, &subject_key, &task_id)
            .await?;
        self.task_bindings
            .merge_metadata(
                &binding.id,
                &serde_json::json!({
                    REQUESTED_SKILLS_KEY: request.skills,
                    REQUESTED_HEAD_KEY: request.head_sha,
                }),
            )
            .await?;
        info!(
            task_id = %task_id,
            loop_id = %rule.id,
            subject_key = %subject_key,
            skills = ?context.skills,
            "Opened a Task for a check-run trigger"
        );

        let binding_id = binding.id.clone();
        moatless_loops::adapters::subject_events::route_subject_event(
            &self.routing,
            connection,
            rule,
            &message,
            moatless_loops::adapters::subject_events::SubjectEvent {
                adapter_label: ADAPTER_LABEL,
                binding_provider: ADAPTER,
                subject_kind: SUBJECT_KIND,
                subject_key: &subject_key,
                new_task_binding_id: Some(&binding_id),
                bindings: vec![binding],
                made_by: None,
            },
        )
        .await?;
        Ok(())
    }

    /// Send the open Task one turn for a repeat trigger that names a new head
    /// or a skill it lacks; drop one that names neither, a redelivery.
    ///
    /// A new head is delivered even with no new skill, rather than left to the
    /// push event: the trigger can follow the Task's own handling of that push,
    /// as when a gate raises the tier and dismisses the approval.
    async fn deliver_repeat_trigger(
        &self,
        rule: &SubscriptionMatch,
        binding: &TaskBinding,
        request: &Execution,
    ) -> Result<(), AdapterError> {
        let Some((fresh, new_head)) = repeat_delta(binding, &rule.skills, request) else {
            debug!(task_id = %binding.task_id, head = %request.head_sha, "Check-run trigger dropped: same head and no new skill");
            return Ok(());
        };
        if self.current_pr(rule, request).await?.is_none() {
            return Ok(());
        }
        let text = repeat_text(request, &fresh, new_head);
        let added = (!fresh.is_empty()).then_some(fresh.as_slice());
        let message = self.trigger_message(rule, request, "", text, added).await;
        // Delivered before the head and skills are recorded: recorded first, a
        // failed delivery's retry finds nothing new and drops the turn. A
        // repeat delivery is deduped on the event id, which names both.
        self.routing
            .deliver_with_skills(rule, binding, &message, ADAPTER_LABEL, fresh.clone())
            .await?;
        let mut stored = requested_skills(binding);
        stored.extend(fresh.iter().cloned());
        self.task_bindings
            .merge_metadata(
                &binding.id,
                &serde_json::json!({
                    REQUESTED_SKILLS_KEY: stored,
                    REQUESTED_HEAD_KEY: request.head_sha,
                }),
            )
            .await?;
        info!(task_id = %binding.task_id, head = %request.head_sha, skills = ?fresh, "Delivered a repeat check-run trigger to its Task");
        Ok(())
    }

    /// A check-run trigger's turn. `added` names the skills a repeat trigger
    /// grew, keying the event id so each growth is its own turn.
    async fn trigger_message(
        &self,
        rule: &SubscriptionMatch,
        request: &Execution,
        author: &str,
        text: String,
        added: Option<&[String]>,
    ) -> InboundMessage {
        let subject = GithubPrSubject::new(&request.owner, &request.repo, request.pr_number);
        let event_sha = match added {
            Some(skills) => format!("{}:{}", request.head_sha, skills.join(",")),
            None => request.head_sha.clone(),
        };
        let user_id =
            super::auth::resolve_sender_user(author, None, rule, self.users.as_ref()).await;
        InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: rule.source_matcher.clone(),
            thread_id: None,
            message_thread_id: request.pr_number.to_string(),
            user_id,
            author: moatless_types::adapter::author_handle(author),
            sender_external_id: None,
            requester: None,
            text,
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::json!({
                "owner": request.owner,
                "repo": request.repo,
                "pr_number": request.pr_number,
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::check_run_trigger(&subject, &event_sha)),
            source_task_id: None,
            audience: Audience::Everyone,
            replies_to: Vec::new(),
            event: Some(super::config::inbound_event(
                super::config::CHECK_RUN_TRIGGER,
            )),
        }
    }
}

#[cfg(test)]
#[path = "check_run_trigger_tests.rs"]
mod tests;
