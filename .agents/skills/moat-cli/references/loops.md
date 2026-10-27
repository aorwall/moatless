# Repo Loops — `.moatless/loops/` Reference

Instead of creating triggers by hand, a repo can declare its automations in git
under `.moatless/loops/<loop-name>/LOOP.md`. The backend reconciles them into the
loop tables on its own: every two minutes it reads each GitHub repository's
default-branch head, and when the head has moved it syncs that repository's
workspaces (overrides left alone) and then its Loops. A merged `LOOP.md` shows
up in `moat loops list` within a few minutes, inactive. `sync-config` runs the
same Loop sync at once, on request. The YAML **frontmatter is the config**; the markdown
**body is the prompt** (the agent's `append_system_prompt`) — mirroring Claude
Code's `SKILL.md`.

```markdown
---
on: github_pr            # event source; also: schedule, slack, linear
name: Java Review        # required — what the loop is called in the UI and CLI
agentProfile: claude-code   # optional, default claude-code
model: opus              # optional; sync skips the loop if another agentProfile owns it
routingMode: by_subject  # optional, default by_subject
workspace: Java Service  # optional — the workspace name tasks run in
tags: [review]           # optional
skills: [java-review]    # optional — activated on every fresh agent session
memory:                  # optional — summaries of loop-scope entries in each task's prompt
  preload: [review:conventions, review:*]   # keys, or `prefix*`
# optional: sourceMatcher, teamId, eventFilter, adapterConfig, settleAfterHours, closeAfterSettledDays
---

Review this pull request. Post inline comments.
```

```markdown
---
on: schedule
name: Daily Health Check
schedule:
  cron: "0 0 6 * * MON-FRI *"     # 7-field cron (sec min hour dom mon dow year)
  timezone: Europe/Stockholm
  message: "Run the daily platform health check."
---

Run the daily-health-check skill and file a Linear ticket for any regressions.
```

```markdown
---
on: slack
name: CI Failure Triage
sourceMatcher: C0000000001      # the channel id to listen on (required for slack)
teamId: T0000000001             # Slack workspace id; omit if only one slack connection exists
routingMode: ongoing            # one long-lived task instead of one-per-subject
adapterConfig:
  allow_bot_messages: true
  trigger: all_messages
---

Monitor this channel for CI failures and triage them in-thread.
```

```markdown
---
on: linear
name: Ready Pool
sourceMatcher: <linear-team-uuid>   # the Linear TEAM whose issues to watch (required)
teamId: <linear-org-id>             # Linear workspace/org id; omit if only one linear connection exists
triggerStates: [Ready]              # spawn a task when an issue ENTERS one of these state names
startState: In Progress             # optional — move the issue to this state when the task is picked up
projectIds: [<project-uuid>]        # optional — only fire for issues in these projects
requireLabels: [agent]              # optional — issue must carry all of these labels
---

Pick up this Linear ticket, implement it, and open a PR linked back to the issue.
```

```bash
moat repos sync-config <repo-id>   # sync now instead of waiting (admin only)
moat loops list                    # synced loops show up here
```

Notes:
- `on` and `name` are the two required keys. The **directory name** is the sync
  key (rename the directory and you get a new loop); `name` is what the loop is
  displayed as, and can be reworded freely. A `LOOP.md` without a `name` is
  **skipped** with `missing \`name\` field`, and any loop already synced from that
  directory is left untouched — nothing that was firing stops firing.
- The Loop list prints the source beside the name (`Slack · C0000000001`,
  `Schedule · 0 0 6 * * MON-FRI *`), so the name never has to carry the id. What
  reads well depends on `on`: for `slack`, the channel as people say it
  (`#general`); for `github_pr`, the job plus the repo, since nearly every repo
  declares one and the list otherwise fills with identical rows (`PR review —
  soaplabs/my-service`); for `schedule`, the job rather than the cron (`Daily
  Health Check`); for `linear`, what the loop picks up (`Ready Pool`). A name like
  `Slack C0000000001` is the one to avoid — it stutters against the line below it
  and goes stale the moment the channel changes.
- Synced loops are tracked with `config_source = 'git'`, keyed by
  `(repository, loop dir name)`. Each sync upserts them and removes loops whose
  directory was deleted. Manually-created loops are never touched.
- A sync whose fetch fails changes nothing: no loop is deactivated or removed.
  When the automatic sync's Loop or workspace half fails, it retries the same
  head on its next pass. A loop it skips (bad frontmatter, unknown workspace,
  missing connection) is logged at WARN with the reason; `sync-config` returns
  the same reasons in its summary.
- The automatic sync runs only when the head moves, so a loop skipped for a
  reason fixed outside git (a connection added, a workspace created by hand)
  lands at the next push. Run `sync-config` to land it now.
- A repository whose creator has no GitHub credential is read unauthenticated,
  and only every 30 minutes: GitHub allows 60 such requests an hour per IP.
- **An overridden loop stops reading its `LOOP.md`, and editing that file then
  changes nothing that runs.** `moat loops override --id <id>` (or Override in
  the UI) detaches a synced loop so an administrator's local edits survive the
  next `sync-config`. The loop keeps its `sourceConfigPath`, so the file still
  looks like the loop's configuration while sync skips it. `moat loops list`
  prints a CONFIG column — `git`, `manual`, or `manual (overridden)` — and names
  the overridden rows with their paths below the table. Check it before you edit
  a `LOOP.md`, and re-attach with `moat loops reset-git --id <id>`, which
  discards the local edits and takes the file again. `reset-git` also restores a
  loop an administrator tombstoned.
- `github_pr` loops need a pre-existing GitHub **connection** for the org (it
  holds the webhook secret and can't be committed to git). With no connection the
  loop is **skipped** with a reason, not errored. Schedule loops need no connection.
- A `github_pr` loop with `adapterConfig.create_on: check_run` opens its task
  only when a trusted GitHub App completes a matching check run, never on PR
  open. The check run must match all of `check_run.names` (required),
  `check_run.app_ids` (required) and `check_run.conclusions` (optional; absent
  accepts any). Sync skips, and the Loop API refuses, a loop missing either
  required list. Any other check run takes the ordinary failed-check path; a
  failed run that matches is a trigger instead, so a bound task never gets its
  failed-check notice. Set `conclusions` to keep failures on that path. Its `output.text` may carry an
  optional marker, every field optional:
  `<!-- moatless-request:v1 {"pr":7,"head":"<sha>","skills":[...],"matched":[{"id":..,"skills":[..]}],"message":"..."} -->`.
  `pr` names the PR, else the check run's first `pull_requests` entry does
  (GitHub leaves that empty for a fork's head). `head`, when given, must equal
  the check run's head. An unreadable marker is ignored and logged. A trigger
  is dropped unless the PR is open, not a draft, and still at that head. The
  first turn is the PR-opened text plus the check's conclusion and output and
  the marker's `message`. The marker's skills join the loop's `skills`; a
  later trigger with new skills sends the open task one turn naming them.
  Once the task has closed, the next trigger opens a new one. Two keys are
  allowed only with `create_on: check_run`: `sourceMatcher: <owner>/*` serves
  every repo of the connection's org that has no exact loop of its own, and
  `workspace: auto` runs each task in the repo's Workspace (the loop's own
  when it holds the repo), registering the repo and copying a Workspace from
  `provision.workspace_template` when it has none. Ignore triggers with
  `ignored_events: [check_run_trigger]`. A review gate is the example:

  ```yaml
  on: github_pr
  name: gate-review
  sourceMatcher: soaplabs/*
  workspace: auto
  skills: [pr-review]
  adapterConfig:
    create_on: check_run
    check_run:
      names: [soap-review-request]
      app_ids: [5421124]
      conclusions: [neutral]
    provision:
      workspace_template: review-template
  ```
- `slack` loops resolve their **connection** from `teamId` (the Slack workspace id,
  e.g. `T0000000001`); if `teamId` is omitted and exactly one slack connection
  exists it's used, otherwise the loop is **skipped** with a reason. `sourceMatcher`
  (the channel id) is **required** — there's no `owner/repo` default. Pair with
  `routingMode: ongoing` for a single long-lived triage task.
- `linear` loops listen for **issue status transitions** and spawn a task when an
  issue ENTERS one of `triggerStates` (matched against Linear's state **names**).
  This is opt-in: the synced trigger sets `create_on_status_entry: true`
  automatically, so existing mention/assignment Linear rules are unaffected.
  `sourceMatcher` is the Linear **team UUID** (the listen scope) and is
  **required** — there's no `owner/repo` default. The **connection** resolves from
  `teamId` (the Linear **workspace/organization id**); omit it when exactly one
  Linear connection is installed, otherwise the loop is **skipped**. `projectIds`
  and `requireLabels` further narrow which issues fire. `startState` (optional)
  moves the issue into that workflow state (e.g. `In Progress`) when the task is
  picked up, so the board reflects the agent took it — resolved within the
  issue's team and skipped if the issue is already there. When the issue reaches
  a terminal state the spawned task is auto-closed; `adapterConfig.terminal_states`
  (default `completed`, `canceled`) matches a state type or a state name,
  case-insensitively. A task still open when its issue re-enters a trigger
  state is told so, with who moved it, instead of a second task starting; every
  task bound to the whole issue hears each state change.
  `adapterConfig.bind_issue: false` binds nothing to the issue: the task hears
  no later comment or state change (only replies to a comment it posts), each
  firing starts a fresh task, and a terminal state closes none. Use it for a
  loop that triages a ticket and hands it to another loop's task.
  `adapterConfig.trigger_labels` (label names, case-insensitive) also fires the
  loop when an issue is created carrying one or an update adds one; when the
  loop lists `triggerStates`, the issue must also sit in one of them. A loop with
  `trigger_labels` fires on status entry only for the `triggerStates` it lists,
  never the default. `adapterConfig.exclude_labels` stops every trigger,
  mention and delegation included, for an issue carrying one of them.
  `routingMode: ongoing` sends each status-entry or label event, as a message
  naming the issue, its state, URL and the reason, to the loop's one ongoing
  task (opened by the first event) and binds nothing to the issue.
  The same knobs work on a hand-made trigger:
  `moat loops create-subscription --adapter linear --connection-id <id>
  --name "Ready Pool" --workspace-id <workspace-id> --source-matcher <team-uuid> --adapter-config
  '{"create_on_status_entry":true,"trigger_states":["Ready"],"project_ids":["<proj>"],"start_state":"In Progress"}'`.
- `workspace` names the Workspace every task the loop spawns runs in, by its
  display name. It must be a Workspace this repository is placed in, or the loop
  is **skipped** with `no workspace named \`<name>\` holds this repository`. Omit
  it and sync resolves the repository's own Workspace, primary placement first; a
  repository placed in none is **skipped**.
- `skills` lists skill names by directory name under `.claude/skills/` or
  `.agents/skills/`. Every task the loop spawns activates them: the sandbox reads
  each `SKILL.md` and frontloads its body ahead of the prompt on each fresh agent
  session, so a restart does not lose them. Names are not validated at sync time —
  a skill exists on the pod that runs the task, not in the database — and one that
  resolves nowhere is logged and skipped.
- **A `LOOP.md` cannot make a loop run.** Git carries configuration only: a
  synced loop lands inactive with no run-as user, and an administrator has to
  activate it in Moatless (`moat loops activate --id <id> --run-as-user-id <user>`,
  or Activate in the UI) naming the identity its tasks run as. `active` and
  `runAsUserId` in the frontmatter are ignored, and `sync-config` reports each
  one it saw as a warning.
- That approval then **survives every later sync**. Once activated, a loop keeps
  running as its approved user while `sync-config` keeps its configuration
  current — re-approval is only needed for a loop nobody has activated yet, or
  one whose tombstone an administrator restored from Git.
