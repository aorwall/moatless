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
  message: "Cluster the last seven days of open feedback and publish today's approval page."
# runAsUserId and active are approved outside Git. The run-as user must be a
# Moatless admin: the feedback API shows anyone else only their own tasks' items.
#   moat loops activate --id <id> --run-as-user-id <admin user id>
---

# Feedback triage — cluster the inbox and publish the approval page

Agents and people record what cost them time with `moat feedback`, and the
items pile up open. This run takes the last seven days of open feedback,
groups it into clusters that share one root cause, names the repository that owns each fix, and publishes
the clusters on a new approval page for a person to review.

Run the `feedback-triage` skill (`.agents/skills/feedback-triage/SKILL.md`) and
follow it end to end. It owns the window, the clustering rules, the page and
the memory layout.

This Loop is read-only. Do not create a task, do not run `moat feedback status`,
and do not act on any decision saved on a page, even an approval from a Moatless
admin. Decisions wait on the page until approval handling ships.
