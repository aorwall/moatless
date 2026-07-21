# Repo Loops — `.moatless/loops/` Reference

Instead of creating triggers by hand, a repo can declare its automations in git
under `.moatless/loops/<loop-name>/LOOP.md` and have `sync-config` reconcile them
into the trigger tables. The YAML **frontmatter is the config**; the markdown
**body is the prompt** (the agent's `append_system_prompt`) — mirroring Claude
Code's `SKILL.md`.

```markdown
---
on: github_pr            # event source; also: schedule, slack, linear
agentType: claude-code   # optional, default claude-code
routingMode: by_subject  # optional, default by_subject
tags: [review]           # optional
# optional: name, runAsUserId, sourceMatcher, teamId, eventFilter, adapterConfig, active, inactiveTtlDays
---

Review this pull request using the java-review skill. Post inline comments.
```

```markdown
---
on: schedule
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
moat repos sync-config <repo-id>                  # fetch + reconcile loops
moat triggers list --repository-id <repo-id>      # synced loops show up here
```

Notes:
- Synced loops are tracked as triggers with `managed_by = 'repo_loop'`, keyed by
  `(repository, loop dir name)`. Re-running `sync-config` upserts them and removes
  loops whose directory was deleted. Manually-created triggers are never touched.
- `github_pr` loops need a pre-existing GitHub **connection** for the org (it
  holds the webhook secret and can't be committed to git). With no connection the
  loop is **skipped** with a reason, not errored. Schedule loops need no connection.
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
  a terminal state (`completed`/`canceled`) the spawned task is auto-closed.
  The same knobs work on a hand-made trigger:
  `moat triggers create-adapter-event --adapter linear --connection-id <id>
  --source-matcher <team-uuid> --adapter-config
  '{"create_on_status_entry":true,"trigger_states":["Ready"],"project_ids":["<proj>"],"start_state":"In Progress"}'`.
- `runAsUserId` resolves: frontmatter → `MOATLESS_DEFAULT_RUN_AS_USER_ID` →
  repo owner → repo creator.
