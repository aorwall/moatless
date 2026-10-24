---
on: schedule
name: Feedback Triage
workspace: moatless
agentProfile: claude-code
model: opus
tags:
  - feedback-triage
skills: [feedback-triage]
memory:
  preload: [triage:runs]
closeAfterSettledDays: 7          # the run's approval page lives on this task
schedule:
  # 7 fields: sec min hour day-of-month month day-of-week year. 07:00 UTC on
  # weekdays, after the 06:00 Agent CLI Upgrade.
  cron: "0 0 7 * * MON-FRI *"
  timezone: UTC
  message: "Act on saved triage decisions, then cluster the last seven days of open feedback and publish today's approval page."
# runAsUserId and active are approved outside Git. The run-as user must be a
# Moatless admin: the feedback API shows anyone else only their own tasks' items.
#   moat loops activate --id <id> --run-as-user-id <admin user id>
---

# Feedback triage — cluster the inbox, publish the approval page, act on decisions

Agents and people record what cost them time with `moat feedback`, and the
items pile up open. This run acts on the decisions saved on earlier pages,
closes clusters whose fix PR merged, then takes the last seven days of open
feedback, groups it into clusters that share one root cause, names the
repository that owns each fix, publishes the clusters on a new approval
page, and posts a summary with the page link to Slack.

Run the `feedback-triage` skill (`.agents/skills/feedback-triage/SKILL.md`) and
follow it end to end. It owns the window, the clustering rules, the page, the
memory layout and what each decision does.

Act only on a decision from a Moatless admin. A message from a page means a
reader sent their saved decisions: run the skill's steps 1 to 3, not a full run.
