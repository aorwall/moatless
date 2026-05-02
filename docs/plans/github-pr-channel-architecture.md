# GitHub PR Channel Full Refactor Plan

## Goal

Refactor the GitHub PR integration onto the target channel architecture:

1. `provider_connections`
2. `ingress_rules`
3. `task_subscriptions`

This is a target-state refactor plan, not an incremental cleanup plan.

GitHub PR should be the first provider migrated to this model because it is the clearest case where:

- the inbound event feed is not the same thing as the outbound reply target
- task-creation policy is distinct from transport/auth configuration
- self-suppression and multi-task fan-out need explicit task subscriptions

## Position in the Channel Architecture

The feature should still be called "channels" externally.

Internally, GitHub should map onto the shared channel architecture like this:

- `provider_connection`
  - GitHub app installation, org webhook, or repo webhook
  - owns credentials, webhook secret, bot identity, and transport configuration
- `ingress_rule`
  - matches repos or repo patterns under that connection
  - decides which GitHub events may create tasks
  - defines task-creation defaults
- `task_subscription`
  - links a task to one PR
  - provides the canonical outbound reply target
  - carries task-local self-suppression and follow-up delivery metadata

## Why the Current Model Is Hard to Reason About

Today `github_pr` uses the same generic channel abstractions for two different jobs:

- defining how GitHub is connected and authenticated
- deciding whether a webhook event for `owner/repo` should create a task
- deciding which existing tasks should keep receiving events from PR `#123`

That makes the flow harder to follow because:

- provider connection, ingress, and PR-following are different concepts
- self-suppression of task-authored GitHub comments is implicit and scattered
- the current model makes GitHub event-creation policy feel accidental rather than designed

## Confirmed Product Rules

The refactor should encode these rules directly:

- Replies back to channels come only from `moat reply`.
- The backend should not auto-post task completion/error summaries.
- A task that created or follows a PR should receive later relevant PR activity:
  - issue comments on the PR
  - review comments
  - reviews
  - sync/push events
  - CI/build/check events
- A review-created task and a PR-creator task should receive the same event classes.
- If a task produced a GitHub comment, that same task should not receive that comment back as inbound input.
- If a task produced a GitHub comment, sibling tasks subscribed to the same PR may still receive it.
- A single PR should not spawn multiple ingress-created tasks from multiple GitHub rules for the same repo.

## Target Architecture

### Concept 1: GitHub Provider Connection

A provider connection answers:

"How do we receive GitHub events and send GitHub replies for this scope?"

This is transport-scoped, not task-scoped.

Suggested responsibilities:

- identify the GitHub scope the connection serves:
  - org-wide
  - repo-specific
  - installation-specific
- own webhook secret, installation/app identity, bot token fallback, and transport config
- provide the auth and API context used by inbound webhook handling and outbound PR comments

This is the right place for an org-level GitHub webhook that receives PR events for many repos.

### Concept 2: GitHub Ingress Rule

An ingress rule answers:

"For repos and events matching this rule, when should a GitHub event create a task, and what defaults should that task use?"

This is policy-scoped.

Suggested responsibilities:

- point at the provider connection
- match repo scope:
  - exact repo
  - org-wide pattern
  - future project/group patterns for GitLab-style providers
- define which event kinds may create tasks
- define `repository_id`, `task_name`, `prompt_template`, `tag_ids`, `run_as_user_id`, and `agent_type`
- define PR/task lifecycle policy such as close-on-close and success notifications

Because the intended product rule is "one PR should not spawn multiple ingress-created tasks from multiple rules", the effective rule set must resolve to at most one ingress-created task per PR.

### Concept 3: GitHub PR Task Subscription

A task subscription answers:

"Task `T` is following PR `#123` in repo `owner/repo`, so deliver later relevant PR events to it and use this PR as the outbound reply target."

This is task-scoped.

Suggested responsibilities:

- bind `task_id` to `repo` + `pr_number`
- store reply metadata needed for outbound comments
- point back to the provider connection used for transport/auth
- track last-seen provider metadata if needed
- provide the basis for task-local self-suppression

## Recommended Data Model

The refactor should stop forcing GitHub PR behavior through generic `channel_configs` and `channel_threads`.

Instead, implement GitHub on top of the shared target tables:

### `provider_connections`

One row per external provider connection.

Suggested fields:

| Field | Meaning |
| --- | --- |
| `id` | Internal identifier |
| `provider` | `github` |
| `connection_kind` | `org_webhook`, `repo_webhook`, `app_installation`, or equivalent |
| `scope_value` | Org name, repo full name, installation id, or similar |
| `credential_ref` | Token/app identity/install reference |
| `webhook_secret` | Optional secret override |
| `transport_config` | Provider-specific JSON for GitHub transport behavior |
| `created_by` / `scope` / `team_id` | Ownership metadata |

For GitHub, one org-wide connection can receive webhook events for many repos.

### `ingress_rules`

One row per task-creation/routing rule.

Suggested fields:

| Field | Meaning |
| --- | --- |
| `id` | Internal identifier |
| `provider` | `github` |
| `connection_id` | Which GitHub connection feeds this rule |
| `scope_matcher` | Repo matcher such as `owner/repo` or `org:*` |
| `event_filter` | Which GitHub events may create tasks |
| `repository_id` | Internal repository used when creating tasks |
| `prompt_template` | Extra prompt for created tasks |
| `task_name` | Default task name |
| `tag_ids` | Default tags |
| `run_as_user_id` | Fallback user |
| `agent_type` | Agent type |
| `close_task_on_close` | Whether PR close/merge can close tasks |
| `notify_on_success` | Whether successful CI events are emitted |
| `active` | Whether the rule is enabled |
| `created_by` / `scope` / `team_id` | Ownership metadata |

The important point is that task-creation defaults belong here, not on the provider connection.

### `task_subscriptions`

One row per task-to-PR subscription.

Suggested fields:

| Field | Meaning |
| --- | --- |
| `id` | Internal identifier |
| `task_id` | Internal task |
| `provider` | `github` |
| `subject_kind` | `github_pr` |
| `subject_key` | Stable key such as `owner/repo#123` |
| `connection_id` | Which GitHub connection to use for transport/auth |
| `reply_target` | Provider-specific data for outbound replies |
| `created_by` | Why or by whom the subscription was created |
| `created_reason` | For example `gh_pr_create`, `review_task`, `manual_connect` |
| `last_event_at` | Activity timestamp |
| `metadata` | Provider-specific extras |

Unique constraint:

- `(task_id, provider, subject_kind, subject_key)`

This keeps subscriptions idempotent without preventing multiple tasks from following the same PR.

## Event Model

The refactor should normalize GitHub webhook payloads into an explicit internal event type before any routing or delivery happens.

Suggested model:

```rust
enum GitHubPrEventKind {
    PrOpened,
    PrReopened,
    PrSynchronized,
    IssueCommentCreated,
    ReviewCommentCreated,
    ReviewSubmitted,
    CheckRunFailed,
    CheckSuiteSucceeded,
    PrClosed,
}

struct GitHubPrEvent {
    connection_id: String,
    repo_full_name: String,
    pr_number: u64,
    kind: GitHubPrEventKind,
    external_id: String,
    authored_by_task: bool,
    authored_task_id: Option<String>,
    text: Option<String>,
    thread_properties: serde_json::Value,
}
```

The key addition is that the event model should distinguish:

- task-authored events
- human/CI events

And when task-authored, it should identify the task that produced the event when possible.

## Target Event Flow

For each inbound GitHub PR webhook:

1. Parse and verify the webhook.
2. Resolve the matching GitHub `provider_connection`.
3. Normalize the payload into a `GitHubPrEvent`.
4. Resolve whether the event was produced by a task.
5. Load all `task_subscriptions` for that `repo_full_name + pr_number`.
6. While delivering:
   - send the event to every subscribed task
   - except the originating task when `authored_by_task = true` and `authored_task_id` matches that subscription
7. Evaluate `ingress_rules` under that connection to decide whether this event may create a task.
8. If the event was task-authored, do not create a new task from it.
9. If a matching rule allows task creation and no ingress-created task already exists for that PR, create the task.
10. Immediately create a `task_subscription` for the new task.

This gives the intended behavior:

- existing PR-following tasks continue receiving relevant activity
- a task does not receive its own GitHub comment back
- task-authored comments do not spawn fresh review/comment tasks
- one PR does not create multiple ingress-created tasks from multiple rules

## How to Resolve `authored_by_task`

The current code already writes `channel_external_id` onto task messages for:

- `moat reply` GitHub comments
- `gh pr comment`
- `gh issue comment`
- `gh pr create` opened events

The refactor should make that a first-class lookup step.

Recommended rule:

- if an inbound GitHub event's `external_id` matches a stored outbound task message, treat it as `authored_by_task = true`
- bind that event to the owning `task_id`

This is more precise than global suppression and matches the intended product rule:

- suppress only for the task that produced it
- still allow sibling subscribed tasks to see it when they are following the same PR

## Outbound Behavior

Outbound GitHub PR replies should also stop being treated as just another generic provider send path.

They should be driven by `task_subscriptions`:

- `moat reply` resolves the GitHub subscription for the task and PR
- the subscription provides repo + PR targeting data
- the provider connection provides auth and transport
- the outbound comment is posted
- the resulting GitHub external ID is persisted on the originating task message

That preserves the self-suppression link when the webhook arrives later.

If a task ever has more than one GitHub PR subscription, outbound reply must target a specific subscription rather than deduplicating by repo-level config.

## Removal of the Old Split-Brain Paths

As part of the full refactor:

- remove the unused backend-driven automatic channel notification path for GitHub PRs
- stop treating GitHub transport, ingress policy, and PR subscriptions as one generic config/thread shape
- stop using repo-level config identity as the implicit outbound reply target for PR comments

GitHub PR is the clearest current example of why the target architecture needs explicit provider connections, ingress rules, and task subscriptions.

## API Direction

The refactor can still preserve the high-level "channels" concept externally, but internally GitHub PR should have explicit operations such as:

- create/update provider connection
- create/update ingress rule
- subscribe task to PR
- list subscriptions for a task
- resolve task subscription from `gh pr create`

If needed, these can still sit under `/api/v1/channels/...` while being backed by dedicated GitHub PR storage and logic.

## Full Refactor Scope

The full refactor should include:

1. New shared schema for `provider_connections`, `ingress_rules`, and `task_subscriptions`.
2. GitHub migration onto those tables.
3. A normalized `GitHubPrEvent` internal model.
4. A dedicated GitHub PR dispatcher:
   - normalize
   - attribute to task if task-authored
   - fan out to subscriptions
   - optionally create one ingress-created task
5. A dedicated subscription-based outbound path for GitHub PR replies.
6. Removal of the unused generic auto-notification path.
7. Updated tests for:
   - task-created PR subscription
   - review-created task subscription
   - same-task suppression for self-authored comments
   - no duplicate ingress-created tasks for one PR

## Staged Migration Strategy

The target architecture should stay fixed, but rollout can happen in stages.

The key rule for a staged rollout is:

- keep the old GitHub channel APIs and UI visible during migration
- move GitHub inbound/outbound behavior onto the new model as early as possible
- avoid a long-lived state where both the old and new GitHub routing engines are authoritative

### Phase 1: Introduce the new model beside the legacy tables

Add:

- `provider_connections`
- `ingress_rules`
- `task_subscriptions`

Add compatibility references so migrated GitHub records can still be linked back to legacy channel rows:

- `ingress_rules.legacy_channel_config_id` or an equivalent mapping table
- `task_subscriptions.legacy_channel_thread_id` or an equivalent mapping table

At this stage:

- existing `github_pr` channel configs and channel threads remain the source of truth
- the new tables exist but are not yet authoritative

### Phase 2: Backfill legacy GitHub channels into the new model

For each legacy GitHub channel config:

- create or find a `provider_connection`
- create one `ingress_rule`

For each legacy GitHub `channel_thread`:

- create one `task_subscription` with `subject_kind = github_pr`

Initial migration can preserve the current repo-scoped GitHub behavior by creating one connection per legacy GitHub channel config if needed.

After that works, multiple migrated repo rules can later be consolidated under one org-level GitHub connection without changing task subscriptions.

### Phase 3: Introduce a compatibility adapter for the old channel surface

Keep the "channels" feature and old GitHub-facing APIs, but resolve them through the new model.

Examples:

- listing GitHub channels for a task should project from `task_subscriptions`
- looking up a legacy GitHub config should resolve to its mapped `ingress_rule`
- `moat reply --channel <legacy_config_id>` should resolve that legacy config to the matching GitHub subscription target

At this stage, legacy GitHub rows are still present, but they are compatibility handles rather than the preferred runtime model.

### Phase 4: Move inbound GitHub webhook processing to the new model

Switch GitHub webhook handling to:

- resolve `provider_connection`
- load matching `ingress_rules`
- load `task_subscriptions`
- deliver events and create tasks through the new dispatcher

Legacy `channel_configs` and `channel_threads` should no longer drive GitHub webhook routing at this point.

### Phase 5: Move outbound GitHub replies to the new model

Switch GitHub outbound behavior to resolve the reply target from `task_subscriptions`, not from repo-level channel config identity.

During this phase:

- a task with one GitHub subscription can reply implicitly
- a task with multiple GitHub subscriptions should require an explicit target

This is the point where the most important ambiguity in the current model disappears.

### Phase 6: Make the new model authoritative and freeze legacy GitHub writes

Once inbound and outbound both use the new model:

- stop creating new legacy GitHub `channel_configs` and `channel_threads`
- keep legacy rows readable for compatibility only
- keep projecting GitHub state into the old "channels" UI/API as long as needed

### Phase 7: Remove the legacy GitHub storage path

After UI, API, and migration tooling no longer depend on the old GitHub rows:

- remove legacy GitHub-specific use of `channel_configs`
- remove legacy GitHub-specific use of `channel_threads`
- keep the generic channel tables for Slack/Telegram/webhook/cron until or unless they are migrated too

## How to Keep "GitHub Channels" During Migration

The cleanest approach is to treat legacy GitHub channels as compatibility projections over the new model.

That means:

- the product still shows "GitHub channels"
- old API ids can still resolve
- internally, GitHub logic gradually stops depending on `channel_configs` and `channel_threads`

The practical mapping is:

- legacy GitHub channel config -> `provider_connection` + `ingress_rule`
- legacy GitHub channel thread -> `task_subscription`

This lets GitHub migrate first without forcing Slack or Telegram to change at the same time.

## Refactor Stance

This plan should be executed as one coherent architecture change, not as a sequence of partial compatibility layers that preserves the current GitHub-specific behavior inside generic channel tables.

The intended end state is:

- GitHub provider connections are modeled explicitly.
- GitHub ingress rules are modeled explicitly.
- GitHub PR subscriptions are modeled explicitly.
- task-authored event attribution is explicit and part of the main delivery path.
- the old generic GitHub channel/thread interpretation is removed rather than kept as a second path.

## Recommendation

Do the full refactor.

The right target is not "slightly clearer use of `channel_configs` and `channel_threads`". The right target is:

- one explicit provider connection for the org/repo scope
- explicit ingress rules for repo/event matching and task defaults
- explicit task subscriptions to PRs
- explicit task attribution for self-authored GitHub events

That is the architecture that actually matches the intended behavior.
