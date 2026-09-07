# Repo Loops — `.moatless/loops/` Reference

Instead of creating triggers by hand, a repo can declare its automations in git
under `.moatless/loops/<loop-name>/LOOP.md` and have `sync-config` reconcile them
into the trigger tables. The YAML **frontmatter is the config**; the markdown
**body is the prompt** (the agent's `append_system_prompt`) — mirroring Claude
Code's `SKILL.md`.

```markdown
---
on: github_pr            # event source; also: schedule, slack, linear
name: Java Review        # required — what the loop is called in the UI and CLI
agentType: claude-code   # optional, default claude-code
model: opus              # optional; sync skips the loop if another agentType owns it
routingMode: by_subject  # optional, default by_subject
tags: [review]           # optional
skills: [java-review]    # optional — activated on every fresh agent session
# optional: sourceMatcher, teamId, eventFilter, adapterConfig, inactiveTtlDays
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
moat repos sync-config <repo-id>                  # fetch + reconcile loops
moat loops list      # synced loops show up here
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
  `(repository, loop dir name)`. Re-running `sync-config` upserts them and removes
  loops whose directory was deleted. Manually-created loops are never touched, and
  neither is a synced loop an administrator has **overridden** (`moat loops
  override --id <id>`, or Override in the UI) — it keeps its local edits until someone
  explicitly resets it to Git.
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
  `moat loops create-subscription --adapter linear --connection-id <id>
  --name "Ready Pool" --repository-id <repo-id> --source-matcher <team-uuid> --adapter-config
  '{"create_on_status_entry":true,"trigger_states":["Ready"],"project_ids":["<proj>"],"start_state":"In Progress"}'`.
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
