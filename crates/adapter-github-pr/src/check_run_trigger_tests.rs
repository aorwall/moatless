use chrono::Utc;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::access_control::Scope;
use moatless_types::adapter::AdapterKind;
use moatless_types::loops::{RoutingMode, SubscriptionMatch};

use super::*;

struct NoUsers;

#[async_trait::async_trait]
impl ProviderIdentityLink for NoUsers {}

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const GATE_APP: u64 = 5_421_124;

fn review_loop(adapter_config: serde_json::Value) -> SubscriptionMatch {
    SubscriptionMatch {
        name: "gate-review".to_string(),
        id: "tr_gate".into(),
        adapter_kind: AdapterKind::Github,
        connection_id: "pc_test".into(),
        source_matcher: "soaplabs/repo".into(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        workspace_id: "ws-1".into(),
        prompt_template: None,
        task_name: None,
        tag_ids: vec![],
        memory_preload: Vec::new(),
        agent_properties: Default::default(),
        skills: vec!["pr-review".to_string()],
        run_as_user_id: Some("alice".into()),
        agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
        model: None,
        close_after_settled_days: None,
        settle_after_hours: None,
        node_selector: None,
        stop_when_done: None,
        room: None,
        message_buffer_seconds: None,
        allow_direct_send: false,
        adapter_config: Some(adapter_config),
        ongoing_task_id: None,
        close_task_on_close: true,
        notify_on_success: false,
        active: true,
        created_by: "alice".into(),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn gate_config() -> serde_json::Value {
    serde_json::json!({
        "create_on": "check_run",
        "check_run": {
            "names": ["soap-review-request"],
            "app_ids": [GATE_APP],
            "conclusions": ["neutral"],
        },
    })
}

fn marker(head: &str) -> String {
    format!(
        "Review requested.\n\n{MARKER_PREFIX}{} -->",
        serde_json::json!({
            "head": head,
            "skills": ["pr-review-infra", "pr-review-security"],
            "matched": [{"id": "infra", "skills": ["pr-review-infra"]}],
        })
    )
}

fn request_payload(app_id: u64, head_sha: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "completed",
        "repository": {
            "full_name": "soaplabs/repo",
            "default_branch": "main",
            "description": "The repo",
        },
        "sender": {"login": "soap-review-gate[bot]"},
        "check_run": {
            "id": 99,
            "name": "soap-review-request",
            "head_sha": head_sha,
            "status": "completed",
            "conclusion": "neutral",
            "external_id": "review-request:v1",
            "app": {"id": app_id, "name": "soap-review-gate"},
            "output": {"title": "Review requested", "summary": "2 rules matched", "text": text},
            "pull_requests": [{"number": 42}],
        },
    })
}

async fn normalize_with(
    rule: &SubscriptionMatch,
    payload: &serde_json::Value,
) -> crate::webhook::NormalizedWebhook {
    crate::webhook::normalize(
        "check_run",
        payload,
        &NormalizeContext {
            rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: "soap-review-gate[bot]",
            pr_author_login: None,
            frontend_url: "https://app.example.com",
            users: &NoUsers,
        },
    )
    .await
}

async fn primary(rule: &SubscriptionMatch, payload: &serde_json::Value) -> PrimaryWebhook {
    normalize_with(rule, payload)
        .await
        .primary
        .expect("normalizes")
}

async fn trigger(rule: &SubscriptionMatch, payload: &serde_json::Value) -> Execution {
    match primary(rule, payload).await {
        PrimaryWebhook::CheckRunTrigger(execution) => *execution,
        other => panic!("expected a check-run trigger, got {other:?}"),
    }
}

/// A check run that does not match the trigger falls through to the ordinary
/// check-run path, where a non-failure conclusion is not notified.
fn falls_through(primary: &PrimaryWebhook) -> bool {
    matches!(
        primary,
        PrimaryWebhook::Ignore(IgnoredWebhook::NonNotifiableConclusion)
            | PrimaryWebhook::CheckRun(_)
    )
}

#[tokio::test]
async fn a_valid_trigger_carries_the_markers_skills_the_check_and_the_repository() {
    let rule = review_loop(gate_config());
    let execution = trigger(&rule, &request_payload(GATE_APP, HEAD, &marker(HEAD))).await;

    assert_eq!(execution.pr_number, 42);
    assert_eq!(execution.head_sha, HEAD);
    assert_eq!(execution.check_name, "soap-review-request");
    assert_eq!(execution.conclusion, "neutral");
    assert_eq!(execution.output_title, "Review requested");
    assert_eq!(execution.output_summary, "2 rules matched");
    assert_eq!(execution.skills, ["pr-review-infra", "pr-review-security"]);
    assert_eq!(
        execution.matched,
        [MatchedRule {
            id: "infra".to_string(),
            skills: vec!["pr-review-infra".to_string()],
        }]
    );
    assert_eq!(execution.message, None);
    assert_eq!(execution.default_branch.as_deref(), Some("main"));
    assert_eq!(execution.description.as_deref(), Some("The repo"));
}

#[tokio::test]
async fn a_trigger_without_a_marker_uses_the_check_runs_head_and_pr() {
    let rule = review_loop(gate_config());
    let execution = trigger(&rule, &request_payload(GATE_APP, HEAD, "No marker here.")).await;
    assert_eq!(execution.pr_number, 42);
    assert_eq!(execution.head_sha, HEAD);
    assert!(execution.skills.is_empty());
    assert!(execution.matched.is_empty());
    assert_eq!(execution.message, None);
}

#[tokio::test]
async fn a_check_run_from_another_app_falls_through() {
    let rule = review_loop(gate_config());
    assert!(falls_through(
        &primary(&rule, &request_payload(1, HEAD, &marker(HEAD))).await
    ));
}

#[tokio::test]
async fn a_loop_trusting_no_app_lets_every_check_run_fall_through() {
    let rule = review_loop(serde_json::json!({
        "create_on": "check_run",
        "check_run": {"names": ["soap-review-request"]},
    }));
    assert!(falls_through(
        &primary(&rule, &request_payload(GATE_APP, HEAD, &marker(HEAD))).await
    ));
}

/// The failure path is unchanged: a failed run of a trigger-named check is a
/// failed-check notice when its conclusion is not one the trigger accepts.
#[tokio::test]
async fn a_conclusion_outside_conclusions_falls_through_to_the_failure_path() {
    let rule = review_loop(gate_config());
    let mut payload = request_payload(GATE_APP, HEAD, &marker(HEAD));
    payload["check_run"]["conclusion"] = serde_json::json!("failure");
    assert!(matches!(
        primary(&rule, &payload).await,
        PrimaryWebhook::CheckRun(_)
    ));
}

#[tokio::test]
async fn absent_conclusions_accept_any_completed_conclusion() {
    let rule = review_loop(serde_json::json!({
        "create_on": "check_run",
        "check_run": {"names": ["soap-review-request"], "app_ids": [GATE_APP]},
    }));
    let mut payload = request_payload(GATE_APP, HEAD, &marker(HEAD));
    payload["check_run"]["conclusion"] = serde_json::json!("success");
    assert_eq!(trigger(&rule, &payload).await.conclusion, "success");
}

#[tokio::test]
async fn a_head_mismatch_is_ignored() {
    let rule = review_loop(gate_config());
    let other = "fedcba9876543210fedcba9876543210fedcba98";
    assert!(matches!(
        primary(&rule, &request_payload(GATE_APP, HEAD, &marker(other))).await,
        PrimaryWebhook::Ignore(IgnoredWebhook::InvalidCheckRunTrigger)
    ));
}

#[tokio::test]
async fn an_invalid_marker_is_ignored() {
    let rule = review_loop(gate_config());
    for text in [
        format!("{MARKER_PREFIX}{{not json}} -->"),
        format!("{MARKER_PREFIX}{{\"head\": \"{HEAD}\"}}"),
        format!("{MARKER_PREFIX}{{\"skills\": [\"../etc\"]}} -->"),
        format!("{MARKER_PREFIX}{{\"skills\": [\"\"]}} -->"),
    ] {
        assert!(
            matches!(
                primary(&rule, &request_payload(GATE_APP, HEAD, &text)).await,
                PrimaryWebhook::Ignore(IgnoredWebhook::InvalidCheckRunTrigger)
            ),
            "{text}"
        );
    }
}

/// An exact `pr_opened` loop never reads the gate's check run as a trigger: it
/// is a neutral check run, which no task is woken for.
#[tokio::test]
async fn a_pr_opened_loop_ignores_trigger_check_runs() {
    let rule = review_loop(serde_json::json!({}));
    assert!(matches!(
        primary(&rule, &request_payload(GATE_APP, HEAD, &marker(HEAD))).await,
        PrimaryWebhook::Ignore(IgnoredWebhook::NonNotifiableConclusion)
    ));
}

#[tokio::test]
async fn ignoring_check_run_trigger_drops_it_but_ignoring_check_run_does_not() {
    let mut config = gate_config();
    config["ignored_events"] = serde_json::json!(["check_run_trigger"]);
    assert!(matches!(
        primary(
            &review_loop(config),
            &request_payload(GATE_APP, HEAD, &marker(HEAD))
        )
        .await,
        PrimaryWebhook::Ignore(IgnoredWebhook::EventFiltered)
    ));

    let mut config = gate_config();
    config["ignored_events"] = serde_json::json!(["check_run"]);
    assert!(matches!(
        primary(
            &review_loop(config),
            &request_payload(GATE_APP, HEAD, &marker(HEAD))
        )
        .await,
        PrimaryWebhook::CheckRunTrigger(_)
    ));
}

#[tokio::test]
async fn another_check_run_under_a_trigger_loop_normalizes_as_before() {
    let rule = review_loop(gate_config());
    let mut payload = request_payload(GATE_APP, HEAD, &marker(HEAD));
    payload["check_run"]["name"] = serde_json::json!("CI");
    payload["check_run"]["conclusion"] = serde_json::json!("failure");
    assert!(matches!(
        primary(&rule, &payload).await,
        PrimaryWebhook::CheckRun(_)
    ));
}

fn execution_with(message: Option<&str>, matched: Vec<MatchedRule>) -> Execution {
    Execution {
        owner: "soaplabs".to_string(),
        repo: "repo".to_string(),
        pr_number: 42,
        head_sha: HEAD.to_string(),
        check_name: "soap-review-request".to_string(),
        conclusion: "neutral".to_string(),
        output_title: "Review requested".to_string(),
        output_summary: "2 rules matched".to_string(),
        skills: vec!["pr-review-infra".to_string()],
        matched,
        message: message.map(str::to_string),
        agent: None,
        model: None,
        default_branch: None,
        description: None,
    }
}

#[tokio::test]
async fn a_markers_agent_and_model_reach_the_execution() {
    let rule = review_loop(gate_config());
    let text = format!(
        "{MARKER_PREFIX}{} -->",
        serde_json::json!({"head": HEAD, "agent": "codex", "model": "gpt-6.1-sol"})
    );
    let execution = trigger(&rule, &request_payload(GATE_APP, HEAD, &text)).await;
    assert_eq!(execution.agent.as_deref(), Some("codex"));
    assert_eq!(execution.model.as_deref(), Some("gpt-6.1-sol"));
}

fn opus_loop() -> SubscriptionMatch {
    let mut rule = review_loop(gate_config());
    rule.model = Some("opus".to_string());
    rule
}

fn request_for(agent: Option<&str>, model: Option<&str>) -> Execution {
    let mut request = execution_with(None, vec![]);
    request.agent = agent.map(str::to_string);
    request.model = model.map(str::to_string);
    request
}

#[test]
fn a_model_the_loops_agent_offers_replaces_the_loops_model() {
    assert_eq!(
        trigger_agent(&opus_loop(), &request_for(None, Some("sonnet"))),
        (AgentProfile::ClaudeCode, Some("sonnet".to_string()))
    );
}

#[test]
fn a_model_the_loops_agent_does_not_offer_keeps_the_loops_model() {
    for model in [Some("gpt-6.1-sol"), Some("no-such-model"), None] {
        assert_eq!(
            trigger_agent(&opus_loop(), &request_for(None, model)),
            (AgentProfile::ClaudeCode, Some("opus".to_string()))
        );
    }
}

#[test]
fn a_markers_agent_replaces_the_loops_agent_with_its_model() {
    assert_eq!(
        trigger_agent(
            &opus_loop(),
            &request_for(Some("codex"), Some("gpt-6.1-sol"))
        ),
        (AgentProfile::Codex, Some("gpt-6.1-sol".to_string()))
    );
}

#[test]
fn another_agent_never_inherits_the_loops_model() {
    for model in [Some("opus"), None] {
        assert_eq!(
            trigger_agent(&opus_loop(), &request_for(Some("codex"), model)),
            (AgentProfile::Codex, None)
        );
    }
}

#[test]
fn an_unknown_agent_keeps_the_loops_agent() {
    assert_eq!(
        trigger_agent(
            &opus_loop(),
            &request_for(Some("no-such-agent"), Some("sonnet"))
        ),
        (AgentProfile::ClaudeCode, Some("sonnet".to_string()))
    );
}

#[test]
fn an_execution_stored_without_an_agent_or_model_still_loads() {
    let mut stored = serde_json::to_value(execution_with(None, vec![])).unwrap();
    let fields = stored.as_object_mut().unwrap();
    fields.remove("agent");
    fields.remove("model");
    let loaded: Execution = serde_json::from_value(stored).unwrap();
    assert_eq!((loaded.agent, loaded.model), (None, None));
}

#[tokio::test]
async fn a_markers_message_reaches_the_first_turn() {
    let rule = review_loop(gate_config());
    let text = format!(
        "{MARKER_PREFIX}{} -->",
        serde_json::json!({"head": HEAD, "message": "Focus on the migration."})
    );
    let execution = trigger(&rule, &request_payload(GATE_APP, HEAD, &text)).await;
    let first = first_turn_text("## New Pull Request: #42\n", &execution);
    assert_eq!(
        first,
        "## New Pull Request: #42\n\
         \n## Triggered by check: soap-review-request\n\n\
         **Conclusion:** neutral\n\
         \n### Output\n**Review requested**\n2 rules matched\n\
         \nFocus on the migration.\n"
    );
}

#[test]
fn the_growth_turn_names_the_check_the_skills_and_the_rules() {
    let infra = MatchedRule {
        id: "infra".to_string(),
        skills: vec!["pr-review-infra".to_string()],
    };
    let skills = ["pr-review-infra".to_string()];
    assert_eq!(
        growth_text(
            &execution_with(Some("Look at Terraform."), vec![infra]),
            &skills
        ),
        "The soap-review-request check added pr-review-infra for 0123456 (rule infra).\n\n\
         Look at Terraform."
    );
    assert_eq!(
        growth_text(&execution_with(None, vec![]), &skills),
        "The soap-review-request check added pr-review-infra for 0123456."
    );
}

#[test]
fn new_skills_keeps_request_order_and_drops_held_and_repeated() {
    let requested = ["b", "a", "b", "c"].map(String::from);
    let held = ["a".to_string()];
    assert_eq!(new_skills(&requested, &held), ["b", "c"]);
    assert!(new_skills(&["a".to_string()], &held).is_empty());
}

#[test]
fn rules_asking_for_names_only_the_rules_with_those_skills() {
    let matched = vec![
        MatchedRule {
            id: "infra".to_string(),
            skills: vec!["pr-review-infra".to_string()],
        },
        MatchedRule {
            id: "docs".to_string(),
            skills: vec!["pr-review-docs".to_string()],
        },
    ];
    assert_eq!(
        rules_asking_for(&matched, &["pr-review-docs".to_string()]).as_deref(),
        Some("docs")
    );
    assert_eq!(rules_asking_for(&[], &["x".to_string()]), None);
}

fn marker_for_pr(head: &str, pr: u64) -> String {
    format!(
        "{MARKER_PREFIX}{} -->",
        serde_json::json!({"head": head, "pr": pr, "skills": ["pr-review-infra"]})
    )
}

/// A fork's head leaves `pull_requests` empty; a shared head lists several.
#[tokio::test]
async fn the_markers_pr_wins_over_the_check_runs_pull_requests() {
    let rule = review_loop(gate_config());
    let mut payload = request_payload(GATE_APP, HEAD, &marker_for_pr(HEAD, 7));
    payload["check_run"]["pull_requests"] = serde_json::json!([]);
    assert_eq!(trigger(&rule, &payload).await.pr_number, 7);

    let shared = request_payload(GATE_APP, HEAD, &marker_for_pr(HEAD, 43));
    assert_eq!(trigger(&rule, &shared).await.pr_number, 43);
}

#[tokio::test]
async fn a_trigger_naming_no_pr_anywhere_is_ignored() {
    let rule = review_loop(gate_config());
    let mut payload = request_payload(GATE_APP, HEAD, &marker(HEAD));
    payload["check_run"]["pull_requests"] = serde_json::json!([]);
    assert!(matches!(
        primary(&rule, &payload).await,
        PrimaryWebhook::Ignore(IgnoredWebhook::InvalidCheckRunTrigger)
    ));
}

fn review_binding(task_id: &str, loop_id: &str, active: bool) -> TaskBinding {
    TaskBinding {
        id: format!("tb_{task_id}"),
        task_id: task_id.into(),
        adapter_kind: AdapterKind::GithubPr,
        subject_kind: crate::routing::SUBJECT_KIND.into(),
        subject_key: "soaplabs/repo#42".into(),
        connection_id: "pc_test".into(),
        loop_id: Some(loop_id.into()),
        destination_config: serde_json::json!({}),
        metadata: serde_json::json!({}),
        last_external_event_id: None,
        origin: crate::routing::ORIGIN_ADAPTER_EVENT.into(),
        created_at: Utc::now(),
        last_event_at: Utc::now(),
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active,
        deactivated_at: None,
        watching: true,
    }
}

/// A closed Task does not block the next trigger: its Binding is
/// passed over, so the trigger opens a new Task through the claim takeover.
#[test]
fn a_closed_tasks_binding_is_passed_over() {
    assert!(
        open_trigger_binding(vec![review_binding("closed", "tr_gate", false)], "tr_gate").is_none()
    );
    let found = open_trigger_binding(
        vec![
            review_binding("closed", "tr_gate", false),
            review_binding("other-loop", "tr_other", true),
            review_binding("reopened", "tr_gate", true),
        ],
        "tr_gate",
    );
    assert_eq!(found.map(|b| b.task_id).as_deref(), Some("reopened"));
}
