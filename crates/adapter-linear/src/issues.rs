//! What an issue changing state, or gaining a label, does.
//!
//! Every bound Task hears the issue moved; a configured state or label spawns a
//! Task with no Linear session behind it, or reaches a Loop's ongoing Task; a
//! terminal state may close the Tasks it produced (see [`close`](super::close)).
//! A state change is named by the issue plus Linear's clock, not the state, so
//! re-entering a state is a second change rather than a duplicate.

use std::collections::HashSet;
use std::fmt::Write as _;

use tracing::{debug, info, warn};

use moatless_core::config::Config;

use moatless_adapter_model::binding::{ADAPTER_EVENT_BINDING_ORIGIN, BindScope};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::TaskFinisher;
use moatless_ports::task::TaskOpener;
use moatless_ports::task::TaskReader;
use moatless_ports::task::{EventSubject, InboundTurn, MessageAcceptor, TurnOrigin};
use moatless_types::adapter::{AdapterConnection, TaskBinding};
use moatless_types::loops::{RoutingMode, SubscriptionMatch};
use moatless_types::task::Visibility;

use super::active_turns::ActiveLinearTurns;
use super::agent_api::{LinearAgentApi, LookupKind};
use super::close::close_active_bindings_for_issue;
use super::config::LinearRuleConfig;
use super::delivery::{IssueLabeledDelivery, IssueUpdateDelivery, LabelTrigger};
use super::subject::{ADAPTER, SUBJECT_KIND_SESSION as SUBJECT_KIND};
use super::webhook_payloads::{IssuePayload, LinearActor, LinearLabelRef, actor_handle};

const ADAPTER_LABEL: &str = moatless_types::adapter::AdapterKind::Linear.as_str();

/// Subject kind of the creation claim on a Loop's ongoing Task, keyed by the
/// Loop id. No Binding carries it.
const ONGOING_SUBJECT_KIND: &str = "linear_loop";

/// Subject kind of the creation claim on one firing of a Loop that binds no
/// Task to the issue, keyed by [`firing_key`]. No Binding carries it.
const UNBOUND_SUBJECT_KIND: &str = "linear_issue_firing";

/// The downward ports the issue workflows read, borrowed from
/// [`LinearInboundRouter`](super::delivery::LinearInboundRouter).
pub(super) struct IssuePorts<'a> {
    pub(super) task_bindings: &'a Bindings,
    pub(super) tasks: &'a dyn TaskReader,
    pub(super) config: &'a Config,
    pub(super) task_closer: &'a dyn TaskFinisher,
    pub(super) sandbox_waker: &'a dyn SandboxWaker,
    pub(super) linear_active_turns: &'a ActiveLinearTurns,
    pub(super) task_creation: &'a dyn TaskOpener,
    pub(super) message_send: &'a dyn MessageAcceptor,
    pub(super) routing: &'a RoutingService,
}

/// What [`provision_issue_task`] did.
#[derive(Debug, PartialEq, Eq)]
enum Provisioned {
    Created(String),
    /// The `(issue, Loop)` pair already has an active Task, or, for a Loop
    /// binding no Task, this firing already started one.
    Existing,
}

/// Handle an `Issue.update` payload.
///
/// On a real state transition, notifies every bound task; if the new state
/// is terminal and `close_task_on_close` is set, also runs the close path.
/// `transition_id` is the semantic transition, falling back to `delivered_at`
/// for older records.
pub(super) async fn handle_issue_update(
    ports: &IssuePorts<'_>,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    update: &IssueUpdateDelivery,
) -> Result<(), AdapterError> {
    let issue = &update.issue;
    let prev_state_id = update
        .updated_from
        .as_ref()
        .and_then(|v| v.get("stateId"))
        .and_then(serde_json::Value::as_str);
    let new_state_id = issue.state.as_ref().map(|s| s.id.as_str());

    if prev_state_id.is_none() || new_state_id.is_none() || prev_state_id == new_state_id {
        return Ok(());
    }

    // Persist the new workflow state onto every binding for this issue — for
    // every transition, not just terminal ones. Best-effort.
    persist_issue_status(ports.task_bindings, connection, issue).await;

    let Some(rule) = rules.first() else {
        debug!(issue = %issue.identifier, "No Linear rule found for team — ignoring update");
        return Ok(());
    };
    let rule_config = LinearRuleConfig::from_rule(rule);

    let transition_id = update
        .transition_id
        .clone()
        .unwrap_or_else(|| update.delivered_at.to_string());

    // Forward a state-change notification to every bound task (best-effort).
    notify_bindings_of_state_change(
        ports,
        connection,
        rules,
        issue,
        &transition_id,
        update.actor.as_ref(),
    )
    .await;

    if rule_config.close_task_on_close
        && rule_config.is_terminal(issue.state_kind(), issue.state_name())
    {
        close_active_bindings_for_issue(
            ports.task_bindings,
            ports.tasks,
            ports.config,
            ports.task_closer,
            api,
            rule,
            issue,
        )
        .await?;
    }

    // Independent of the notify/close paths above, which only touch
    // already-bound tasks.
    create_tasks_for_status_entry(ports, api, connection, rules, issue, &transition_id).await
}

/// Act on labels added to an issue, or an issue created with labels: fire every
/// rule whose `trigger_labels` names one of them.
///
/// Fails, so the record retries, only when the label names can't be read or
/// [`must_retry`]; other per-rule failures are logged.
pub(super) async fn handle_issue_labeled(
    ports: &IssuePorts<'_>,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    labeled: &IssueLabeledDelivery,
    delivery_event_id: &str,
) -> Result<(), AdapterError> {
    let issue = &labeled.issue;
    let candidates: Vec<(&SubscriptionMatch, LinearRuleConfig)> = rules
        .iter()
        .map(|r| (r, LinearRuleConfig::from_rule(r)))
        .filter(|(_, cfg)| !cfg.trigger_labels.is_empty())
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }

    let facts = if issue.labels.is_empty() {
        IssueFacts::fetch(api, issue).await?
    } else {
        IssueFacts::from_payload(issue)
    };
    let added: Vec<String> = facts
        .labels
        .iter()
        .filter(|l| labeled.added_label_ids.contains(&l.id))
        .map(|l| l.name.clone())
        .collect();

    let state_name = issue.state_name();
    for (rule, cfg) in candidates {
        let Some(label) = cfg.trigger_label_in(&added) else {
            continue;
        };
        if cfg.is_terminal(issue.state_kind(), state_name)
            || !cfg.label_trigger_allows_state(state_name)
        {
            debug!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                state = state_name,
                "Linear label trigger skipped: issue state not admitted"
            );
            continue;
        }
        if let Some(refusal) = facts.refusal(&cfg) {
            debug!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                refusal,
                "Linear label trigger skipped"
            );
            continue;
        }

        let by = actor_handle(labeled.actor.as_ref())
            .map(|name| format!(" by {name}"))
            .unwrap_or_default();
        let (reason, clock) = match labeled.trigger {
            LabelTrigger::Created => (
                format!("was created with the label \"{label}\"{by}"),
                "created".to_string(),
            ),
            LabelTrigger::Added => (
                format!("was labeled \"{label}\"{by}"),
                issue
                    .updated_at
                    .clone()
                    .unwrap_or_else(|| delivery_event_id.to_string()),
            ),
        };
        let event_id = super::external_id::issue_labeled(&issue.id, &clock);
        match fire_rule(
            ports, api, connection, rule, &cfg, issue, &reason, &event_id,
        )
        .await
        {
            Ok(()) => {}
            Err(e) if must_retry(rule, &e) => return Err(e),
            Err(e) => warn!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                error = %e,
                "Failed to act on Linear label trigger"
            ),
        }
    }
    Ok(())
}

/// The issue's project and labels, as the rule filters read them.
struct IssueFacts {
    project_id: Option<String>,
    labels: Vec<LinearLabelRef>,
}

impl IssueFacts {
    fn from_payload(issue: &IssuePayload) -> Self {
        Self {
            project_id: issue.resolved_project_id().map(str::to_string),
            labels: issue.labels.clone(),
        }
    }

    async fn fetch(api: &LinearAgentApi, issue: &IssuePayload) -> Result<Self, AdapterError> {
        let detail = api.get_issue(&issue.id).await?;
        Ok(Self::from_graphql(&detail, issue))
    }

    /// Falls back to the webhook's project when the fetched issue names none.
    fn from_graphql(detail: &serde_json::Value, issue: &IssuePayload) -> Self {
        let labels = detail
            .pointer("/labels/nodes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Self {
            project_id: detail
                .pointer("/project/id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| issue.resolved_project_id().map(str::to_string)),
            labels,
        }
    }

    fn label_names(&self) -> Vec<String> {
        self.labels.iter().map(|l| l.name.clone()).collect()
    }

    /// Why `cfg` refuses this issue, or `None` when it admits it.
    fn refusal(&self, cfg: &LinearRuleConfig) -> Option<&'static str> {
        let names = self.label_names();
        if !cfg.project_allowed(self.project_id.as_deref()) {
            Some("issue project not in allowlist")
        } else if !cfg.labels_allowed(&names) {
            Some("required labels missing")
        } else if cfg.excluded(&names) {
            Some("issue carries an excluded label")
        } else {
            None
        }
    }
}

/// Start `rule`'s work for `issue`: a message to the Loop's ongoing Task when
/// it routes `ongoing`, else a Task bound to the issue.
///
/// `reason` completes "Linear issue X …"; `event_id` names the ongoing message.
#[expect(
    clippy::too_many_arguments,
    reason = "the issue, the rule that fired and the two strings naming why; \
              each is read by one of the two routes"
)]
async fn fire_rule(
    ports: &IssuePorts<'_>,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    cfg: &LinearRuleConfig,
    issue: &IssuePayload,
    reason: &str,
    event_id: &str,
) -> Result<(), AdapterError> {
    if rule.routing_mode == RoutingMode::Ongoing {
        let text = ongoing_text(issue, reason);
        return deliver_to_ongoing_task(ports, connection, rule, issue, &text, event_id).await;
    }
    match provision_issue_task(ports, api, connection, rule, cfg, issue, event_id).await? {
        Provisioned::Created(task_id) => info!(
            task_id = %task_id,
            issue = %issue.identifier,
            subscription_id = %rule.id,
            reason,
            "Created task from Linear issue trigger"
        ),
        // A re-entered trigger state reaches these through
        // `notify_bindings_of_state_change`.
        Provisioned::Existing => info!(
            issue = %issue.identifier,
            subscription_id = %rule.id,
            "Linear issue already has a task for this trigger or firing — skipping creation"
        ),
    }
    Ok(())
}

/// Whether a failed [`fire_rule`] must fail the record so it retries, rather
/// than be logged. An ongoing Loop's event reaches no Task otherwise: the retry
/// finds the Task through `open_ongoing_task_id` and repeats the same event id.
fn must_retry(rule: &SubscriptionMatch, e: &AdapterError) -> bool {
    rule.routing_mode == RoutingMode::Ongoing || matches!(e, AdapterError::CreationClaimed { .. })
}

/// The message a Loop's ongoing Task gets for one issue event.
fn ongoing_text(issue: &IssuePayload, reason: &str) -> String {
    let mut text = format!(
        "Linear issue {} \"{}\" {reason}.\n\nState: {}",
        issue.identifier,
        issue.title,
        issue.state_name()
    );
    if let Some(url) = issue.url.as_deref().filter(|u| !u.is_empty()) {
        let _ = write!(text, "\n{url}");
    }
    text
}

/// Deliver `text` to `rule`'s ongoing Task, opening that Task first when the
/// Loop has none open. Binds nothing to the issue: the Loop owns the Task.
async fn deliver_to_ongoing_task(
    ports: &IssuePorts<'_>,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    issue: &IssuePayload,
    text: &str,
    event_id: &str,
) -> Result<(), AdapterError> {
    let turn = |task_id: &str, resume: bool| InboundTurn {
        task_id: task_id.to_string(),
        text: text.to_string(),
        file_ids: Vec::new(),
        resume,
        origin: TurnOrigin {
            event_id: Some(event_id.to_string()),
            adapter: Some(ADAPTER_LABEL.to_string()),
            subject_key: Some(issue.id.clone()),
            source_name: Some(rule.source_matcher.clone()),
            author: None,
            sender_id: None,
        },
        sent_by: None,
        skills: Vec::new(),
    };

    if let Some(task_id) = ports.routing.open_ongoing_task_id(rule).await {
        ports
            .message_send
            .accept(turn(&task_id, true))
            .await
            .map_err(|e| AdapterError::task_handoff("deliver to the ongoing task", e))?;
        if let Err(e) = ports.sandbox_waker.wake(&task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to wake the ongoing task");
        }
        return Ok(());
    }

    ports
        .routing
        .claim_task_creation(
            connection,
            rule,
            ONGOING_SUBJECT_KIND,
            &rule.id,
            ADAPTER_LABEL,
            Some(event_id),
        )
        .await?;
    // Named after the Loop, not the issue that happened to open it.
    let named = SubscriptionMatch {
        task_name: rule.task_name.clone().or_else(|| Some(rule.name.clone())),
        ..rule.clone()
    };
    let task_id = match insert_event_task(
        ports.task_creation,
        &named,
        &issue.identifier,
        &issue.title,
        text,
        None,
    )
    .await
    {
        Ok(task_id) => task_id,
        Err(e) => {
            ports
                .routing
                .release_task_creation(&rule.id, ONGOING_SUBJECT_KIND, &rule.id)
                .await?;
            return Err(e);
        }
    };
    ports.routing.set_ongoing_task(&rule.id, &task_id).await?;
    ports
        .routing
        .record_created_task(&rule.id, ONGOING_SUBJECT_KIND, &rule.id, &task_id)
        .await?;
    ports
        .message_send
        .accept(turn(&task_id, false))
        .await
        .map_err(|e| AdapterError::task_handoff("deliver to the ongoing task", e))?;
    info!(
        task_id = %task_id,
        subscription_id = %rule.id,
        issue = %issue.identifier,
        "Opened the Loop's ongoing task from a Linear issue event"
    );
    Ok(())
}

/// Fire every rule opted into `create_on_status_entry` whose `trigger_states`
/// include the new state, applying project/label/exclude filters.
///
/// Fetches the issue once a rule filters on labels or a project the webhook
/// omits, else uses the payload. Best-effort per rule, except [`must_retry`].
async fn create_tasks_for_status_entry(
    ports: &IssuePorts<'_>,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    issue: &IssuePayload,
    transition_id: &str,
) -> Result<(), AdapterError> {
    let state_name = issue.state_name();
    let candidates: Vec<(&SubscriptionMatch, LinearRuleConfig)> = rules
        .iter()
        .map(|r| (r, LinearRuleConfig::from_rule(r)))
        .filter(|(_, cfg)| cfg.creates_on_status(state_name))
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }

    let needs_labels = issue.labels.is_empty()
        && candidates
            .iter()
            .any(|(_, cfg)| !cfg.require_labels.is_empty() || !cfg.exclude_labels.is_empty());
    let needs_project_fetch = issue.resolved_project_id().is_none()
        && candidates
            .iter()
            .any(|(_, cfg)| !cfg.project_ids.is_empty());

    let facts = if needs_labels || needs_project_fetch {
        match IssueFacts::fetch(api, issue).await {
            Ok(facts) => facts,
            Err(e) => {
                warn!(
                    issue = %issue.identifier,
                    error = %e,
                    "Failed to fetch Linear issue for status-entry filtering; using webhook payload"
                );
                IssueFacts::from_payload(issue)
            }
        }
    } else {
        IssueFacts::from_payload(issue)
    };

    let reason = format!("entered \"{state_name}\"");
    let event_id = super::external_id::issue_state(&issue.id, transition_id);
    for (rule, cfg) in candidates {
        if let Some(refusal) = facts.refusal(&cfg) {
            debug!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                refusal,
                "Linear status-entry skipped"
            );
            continue;
        }
        match fire_rule(
            ports, api, connection, rule, &cfg, issue, &reason, &event_id,
        )
        .await
        {
            Ok(()) => {}
            Err(e) if must_retry(rule, &e) => return Err(e),
            Err(e) => warn!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                error = %e,
                "Failed to act on Linear status entry"
            ),
        }
    }
    Ok(())
}

/// Whether `binding` is a live Task `rule` already started on this connection.
fn is_open_for_rule(binding: &TaskBinding, connection_id: &str, rule_id: &str) -> bool {
    binding.active
        && binding.connection_id == connection_id
        && binding.loop_id.as_deref() == Some(rule_id)
}

/// Create a task for `issue` under `rule`, bind it unless `cfg.bind_issue` is
/// off, link it back, and forward the "read the ticket" prompt.
///
/// Idempotent per `(issue, rule)` through [`is_open_for_rule`] when binding,
/// else per [`firing_key`], so an unbound re-entry starts a fresh Task.
async fn provision_issue_task(
    ports: &IssuePorts<'_>,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    cfg: &LinearRuleConfig,
    issue: &IssuePayload,
    event_id: &str,
) -> Result<Provisioned, AdapterError> {
    let firing = (!cfg.bind_issue).then(|| firing_key(issue, event_id));
    if let Some(firing) = firing.as_deref() {
        let claimed = ports
            .routing
            .claim_task_creation_once(
                rule,
                UNBOUND_SUBJECT_KIND,
                firing,
                ADAPTER_LABEL,
                Some(event_id),
            )
            .await?;
        if !claimed {
            return Ok(Provisioned::Existing);
        }
    } else {
        let has_active = ports
            .task_bindings
            .by_destination_field(ADAPTER, super::SUBJECT_KIND_ISSUE, "issue_id", &issue.id)
            .await?
            .into_iter()
            .any(|b| is_open_for_rule(&b, &connection.id, &rule.id));
        if has_active {
            return Ok(Provisioned::Existing);
        }
    }

    let inbound_text = super::text::build_issue_inbound_text(issue);
    let created = insert_event_task(
        ports.task_creation,
        rule,
        &issue.identifier,
        &issue.title,
        &inbound_text,
        None,
    )
    .await;
    if let (Err(_), Some(firing)) = (&created, firing.as_deref()) {
        if let Err(e) = ports
            .routing
            .release_task_creation(&rule.id, UNBOUND_SUBJECT_KIND, firing)
            .await
        {
            warn!(issue = %issue.identifier, error = %e, "Failed to release the firing claim");
        }
    }
    let task_id = match created {
        Ok(id) => id,
        // Preflight failed: surface the reason as an issue comment (this path
        // has no agent session), then bail out — the errored task gets no
        // binding or prompt.
        Err(AdapterError::Shared(SharedAdapterError::AccessDenied(msg))) => {
            if let Err(e) = api
                .create_comment(&issue.id, &format!("Couldn't start this task: {msg}"), None)
                .await
            {
                warn!(issue = %issue.identifier, error = %e, "Failed to post Linear preflight error");
            }
            return Err(SharedAdapterError::AccessDenied(msg).into());
        }
        Err(e) => return Err(e),
    };
    // Open the Linear turn gate so sandbox/agent events on this task surface in
    // Linear's UI. Provisioning already ran inside the creation service.
    ports.linear_active_turns.open(&task_id);

    let team_id = issue.resolved_team_id().unwrap_or_default().to_string();
    let subject_key = if let Some(firing) = firing.as_deref() {
        // Logged rather than returned: a retry would lose the claim and skip.
        if let Err(e) = ports
            .routing
            .record_created_task(&rule.id, UNBOUND_SUBJECT_KIND, firing, &task_id)
            .await
        {
            warn!(task_id = %task_id, error = %e, "Failed to record the firing's task");
        }
        issue.id.clone()
    } else {
        let destination_config = super::subject::issue_destination_config(issue, &team_id);
        ports
            .task_bindings
            .bind(
                connection,
                Some(rule),
                &task_id,
                ADAPTER,
                super::SUBJECT_KIND_ISSUE,
                &issue.id,
                destination_config,
                ADAPTER_EVENT_BINDING_ORIGIN,
                None,
                &BindScope::default(),
            )
            .await?
            .subject_key
    };

    let task_url = super::text::task_url(ports.config, &task_id);
    let attachment_title = super::task_links::task_attachment_title(
        ports.tasks,
        &task_id,
        super::text::ATTACHMENT_TITLE_CHILD,
    )
    .await;
    if let Err(e) = api
        .attachment_link_url(&issue.id, &task_url, &attachment_title)
        .await
    {
        warn!(
            issue = %issue.identifier,
            error = %e,
            "Failed to attach Moatless task URL to Linear issue"
        );
    }

    let external_event_id = super::external_id::issue_created(&issue.id);
    if let Err(e) = ports
        .message_send
        .accept(InboundTurn {
            task_id: task_id.clone(),
            text: inbound_text,
            file_ids: Vec::new(),
            resume: false,
            origin: TurnOrigin {
                event_id: Some(external_event_id),
                adapter: Some(ADAPTER_LABEL.to_string()),
                subject_key: Some(subject_key),
                source_name: Some(rule.source_matcher.clone()),
                author: None,
                sender_id: None,
            },
            sent_by: None,
            skills: Vec::new(),
        })
        .await
    {
        warn!(
            task_id = %task_id,
            issue = %issue.identifier,
            error = %e,
            "Failed to forward Linear issue prompt to task"
        );
    }

    // Move the issue to the configured "started" state so the board reflects
    // pickup. Best-effort and safe against self-retrigger: the resulting
    // `Issue.update` re-enters this path but trips the idempotency guard above.
    // Transitions only when not already in the target state, so a loop via
    // overlapping `start_state`/`trigger_states` doesn't thrash.
    if let Some(start_state) = cfg.start_state.as_deref() {
        if !issue.state_name().eq_ignore_ascii_case(start_state) {
            transition_issue_state(api, &team_id, &issue.id, &issue.identifier, start_state).await;
        }
    }

    Ok(Provisioned::Created(task_id))
}

/// The issue plus the `updatedAt` of the mutation that fired, so a state entry
/// and a label added in one update start one Task, not two.
fn firing_key(issue: &IssuePayload, event_id: &str) -> String {
    format!(
        "{}:{}",
        issue.id,
        issue.updated_at.as_deref().unwrap_or(event_id)
    )
}

/// Move the issue to `state_name`; best-effort, so failures are only logged.
pub(super) async fn transition_issue_state(
    api: &LinearAgentApi,
    team_id: &str,
    issue_id: &str,
    issue_identifier: &str,
    start_state: &str,
) {
    if team_id.is_empty() {
        warn!(
            issue = %issue_identifier,
            "Cannot move Linear issue to start state: missing team id"
        );
        return;
    }
    match api
        .lookup(LookupKind::State, start_state, Some(team_id))
        .await
    {
        Ok(Some(state)) => {
            match api
                .update_issue(issue_id, serde_json::json!({ "stateId": state.id }))
                .await
            {
                Ok(_) => info!(
                    issue = %issue_identifier,
                    start_state,
                    "Moved Linear issue to start state on pickup"
                ),
                Err(e) => warn!(
                    issue = %issue_identifier,
                    start_state,
                    error = %e,
                    "Failed to move Linear issue to start state"
                ),
            }
        }
        Ok(None) => warn!(
            issue = %issue_identifier,
            start_state,
            team_id,
            "Start state not found in the issue's team — leaving issue state unchanged"
        ),
        Err(e) => warn!(
            issue = %issue_identifier,
            start_state,
            error = %e,
            "Failed to resolve Linear start state"
        ),
    }
}

/// Persist the issue's workflow state onto every binding tied to it (both
/// `linear_agent_session` and `linear_issue` subject kinds). Writes the
/// dedicated `external_status*` columns. Best-effort: failures are logged.
async fn persist_issue_status(
    task_bindings: &Bindings,
    connection: &AdapterConnection,
    issue: &IssuePayload,
) {
    let status = super::status::linear_external_status(
        issue.state_name(),
        issue.state_kind(),
        Some(issue.title.as_str()),
        issue.url.as_deref(),
    );

    for subject_kind in [SUBJECT_KIND, super::SUBJECT_KIND_ISSUE] {
        let bindings = match super::subject::bindings_on_issue(
            task_bindings,
            connection,
            subject_kind,
            &issue.id,
        )
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(
                    issue = %issue.identifier,
                    subject_kind,
                    error = %e,
                    "Failed to look up bindings for Linear status persistence"
                );
                continue;
            }
        };
        for binding in bindings {
            if let Err(e) = task_bindings
                .record_external_status(
                    &binding.id,
                    status.status.as_deref(),
                    status.state_kind.as_deref(),
                    &status.raw,
                )
                .await
            {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to persist Linear status onto binding"
                );
            }
        }
    }
}

/// Post a state-change message to every Task bound to the issue, through a
/// session or an issue Binding, then wake it so the message actually runs.
///
/// A Task bound under a rule whose trigger states include the new state hears
/// the issue *re-entered* it, as [`provision_issue_task`] starts no second
/// Task. Failures are logged so the close path still runs.
async fn notify_bindings_of_state_change(
    ports: &IssuePorts<'_>,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    issue: &IssuePayload,
    transition_id: &str,
    actor: Option<&LinearActor>,
) {
    let new_state = issue.state_name();
    let external_event_id = super::external_id::issue_state(&issue.id, transition_id);
    let mut notified = HashSet::new();
    // The Task that moved the issue is passed over, as `fan_out` passes over a
    // maker. A failed lookup delivers: hearing its own move costs a turn, and
    // missing someone else's move costs the work.
    match ports
        .routing
        .own_events()
        .made_by(
            &external_event_id,
            EventSubject {
                adapter: ADAPTER_LABEL,
                source_name: &issue.id,
                subject_key: &issue.id,
            },
        )
        .await
    {
        Ok(Some(task_id)) => {
            notified.insert(task_id);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(issue = %issue.identifier, error = %e, "Failed to look up who moved a Linear issue");
        }
    }

    for subject_kind in [SUBJECT_KIND, super::SUBJECT_KIND_ISSUE] {
        let bindings = match super::subject::bindings_on_issue(
            ports.task_bindings,
            connection,
            subject_kind,
            &issue.id,
        )
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(
                    issue = %issue.identifier,
                    subject_kind,
                    error = %e,
                    "Failed to look up bindings for Linear state-change notification"
                );
                continue;
            }
        };

        // A Binding that hears only replies to its Task's comments
        // (`comments::hears_comment`) is not told of a state change.
        // A closed Task's Binding is inactive, and the closed Task would answer
        // the message with a conflict.
        for binding in bindings
            .into_iter()
            .filter(|b| b.active && !super::subject::hears_only_replies_on_issue(b))
        {
            if !notified.insert(binding.task_id.clone()) {
                continue;
            }
            let reentered = subject_kind == super::SUBJECT_KIND_ISSUE
                && binding
                    .loop_id
                    .as_deref()
                    .and_then(|id| rules.iter().find(|r| r.id == id))
                    .is_some_and(|r| LinearRuleConfig::from_rule(r).creates_on_status(new_state));
            let body = state_change_text(issue, actor, reentered);

            if let Err(e) = ports
                .message_send
                .accept(InboundTurn {
                    task_id: binding.task_id.clone(),
                    text: body,
                    file_ids: Vec::new(),
                    resume: true,
                    origin: TurnOrigin {
                        event_id: Some(external_event_id.clone()),
                        adapter: Some(ADAPTER_LABEL.to_string()),
                        subject_key: Some(binding.subject_key.clone()),
                        ..TurnOrigin::default()
                    },
                    sent_by: None,
                    skills: Vec::new(),
                })
                .await
            {
                warn!(
                    task_id = %binding.task_id,
                    issue = %issue.identifier,
                    error = %e,
                    "Failed to forward Linear state change to task"
                );
                continue;
            }

            if let Err(e) = ports.sandbox_waker.wake(&binding.task_id).await {
                warn!(
                    task_id = %binding.task_id,
                    issue = %issue.identifier,
                    error = %e,
                    "Failed to ensure sandbox for Linear state change"
                );
            }
        }
    }
}

/// The state-change message: "moved to", or "re-entered" for a Task that state
/// started.
fn state_change_text(issue: &IssuePayload, actor: Option<&LinearActor>, reentered: bool) -> String {
    let state = issue.state_name();
    let by = actor_handle(actor);
    let mut body = if reentered {
        let mut body = format!("Linear issue {} re-entered \"{state}\"", issue.identifier);
        if let Some(by) = by {
            let _ = write!(body, " (moved by {by})");
        }
        body.push_str(
            ". This task is still open for it, so no new task was started: read the \
             issue's latest comments for why it came back.",
        );
        body
    } else {
        let mut body = format!("Linear issue {} moved to \"{state}\"", issue.identifier);
        if let Some(by) = by {
            let _ = write!(body, " by {by}");
        }
        body.push('.');
        body
    };
    if let Some(ref url) = issue.url {
        let _ = write!(body, "\n\n{url}");
    }
    body
}

/// Build + insert the Task row shared by the session- and issue-initiated
/// Linear creation paths, assign the trigger's tags, and publish the `Open`
/// event.
///
/// Returns the new task id; callers own subsequent provisioning, binding, and
/// inbound-message delivery.
pub(super) async fn insert_event_task(
    task_creation: &dyn TaskOpener,
    rule: &SubscriptionMatch,
    identifier: &str,
    title: &str,
    preview: &str,
    requested_by: Option<String>,
) -> Result<String, AdapterError> {
    // The shared spine's task-creation entry point: preflight, insert, tags,
    // provisioning, and the `Open` event all run inside it. On preflight
    // failure, surfaced here as `AccessDenied` so the caller can post the
    // reason to Linear — the service's own error event is invisible there:
    // no binding exists yet to open a turn gate.
    let config = moatless_adapter_model::loops::subscription_context(rule);
    let outcome = moatless_loops::loops::firing::inbound::create_task_from_loop(
        task_creation,
        &config,
        moatless_loops::loops::firing::inbound::CreateTaskParams {
            name: rule
                .task_name
                .clone()
                .or_else(|| Some(format!("{identifier} — {title}"))),
            description: moatless_loops::bindings::outbound::truncate_text(preview, 200),
            visibility: Visibility::Public,
            origin_prelude: None,
            // This path is the Linear adapter, so it supplies its own reply
            // guidance directly rather than resolving it through the registry.
            reply_instructions: Some(super::adapter::REPLY_INSTRUCTIONS.to_string()),
            related_to: None,
            event_metadata: serde_json::json!({
                "adapter": ADAPTER_LABEL,
                "source_matcher": rule.source_matcher,
            }),
            requested_by,
        },
    )
    .await
    .map_err(|e| match e {
        moatless_ports::task::TaskOpenError::Refused(msg) => {
            SharedAdapterError::AccessDenied(msg).into()
        }
        other => AdapterError::task_handoff("open a task from a provider event", other),
    })?;

    Ok(outcome.task_id)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_types::adapter::AdapterKind;

    use super::{TaskBinding, is_open_for_rule};

    fn issue_binding(loop_id: &str) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: "ts_1".to_string(),
            task_id: "task-1".to_string(),
            adapter_kind: AdapterKind::Linear,
            subject_kind: crate::SUBJECT_KIND_ISSUE.to_string(),
            subject_key: "issue-1".to_string(),
            connection_id: "pc_1".to_string(),
            loop_id: Some(loop_id.to_string()),
            destination_config: serde_json::json!({ "issue_id": "issue-1" }),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
            watching: true,
        }
    }

    /// Closing the Task a rule started deactivates its Binding, so the issue
    /// entering a trigger state again starts a new Task rather than none.
    #[test]
    fn only_a_live_binding_of_the_same_rule_blocks_a_new_task() {
        let mut binding = issue_binding("tr_1");
        assert!(is_open_for_rule(&binding, "pc_1", "tr_1"));
        assert!(!is_open_for_rule(&binding, "pc_1", "tr_2"));
        assert!(!is_open_for_rule(&binding, "pc_2", "tr_1"));
        binding.active = false;
        assert!(!is_open_for_rule(&binding, "pc_1", "tr_1"));
    }
}
