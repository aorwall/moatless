---
name: orchestrate
description: "End-to-end feature orchestration: grill requirements, write spec, render HTML for review, break into tickets, spawn parallel child tasks, review PRs, loop until done. Use when a user wants to plan and execute a non-trivial feature across many tasks."
disable-model-invocation: true
---

# Orchestrate

Drive a feature from idea to merged PRs: clarify → spec → review → tickets → spawn → monitor → code-review → loop.

This skill spawns child Moatless tasks to implement work in parallel. Each child creates a PR and reports back; you run code review on each and advance blocked tickets as their dependencies land.

---

## Phase 1 — Alignment

Run `/grill-with-docs` to sharpen the idea and resolve ambiguity before writing anything. Ask one question at a time; look up codebase facts rather than asking. Record load-bearing decisions as ADRs in `docs/adr/` as they land.

Stop when you and the user share a precise shared picture of what to build.

---

## Phase 2 — Spec

Without re-interviewing, synthesize the conversation into a spec. First explore the codebase to understand the current state; use the domain glossary (`CONTEXT.md`) and existing ADRs throughout.

Sketch the testing seams (fewest possible; prefer the highest seam). Confirm them with the user before writing.

Write the spec to **`/opt/moatless/workspace/specs/<feature-slug>.md`**:

<spec-template>
# <Feature Title>

## Problem Statement

The problem from the user's perspective.

## Solution

The solution from the user's perspective.

## User Stories

A long, numbered list:
1. As a <actor>, I want <feature>, so that <benefit>

Cover all aspects of the feature extensively.

## Implementation Decisions

- Modules to build or modify
- Interface changes
- Architectural decisions
- Schema changes, API contracts
- No file paths or code snippets (exception: prototype snippets that encode a decision more precisely than prose — inline trimmed to the decision-rich parts)

## Testing Decisions

- What makes a good test here (test external behaviour, not implementation)
- Which modules, which seams
- Prior art in the codebase

## Out of Scope

## Further Notes
</spec-template>

Then run `/html-report` to render a **review page** — not a dump of the spec, but what the user needs to say yes or no and give feedback. The full spec is the source of truth on disk; the HTML is what makes it easy to read and approve in two minutes.

<html-review-content>

The review page has four sections — keep each short:

**1. What we're building** — one paragraph, plain language, no jargon. What will exist afterward that doesn't today? What problem does it solve for the user?

**2. What's in / what's out** — two short bullet lists side by side. "In scope" is the committed surface; "out of scope" sets expectations on what won't be touched.

**3. Key decisions** — 3–6 bullet points covering only the choices that would surprise the user or that they'd want to override. Skip anything routine or obvious. Each bullet is one sentence: the decision, not the reasoning.

**4. Proposed work breakdown** — before writing this section, draft the ticket breakdown: vertical slices, each demoable on its own, with blocking edges. Show it as a Mermaid dependency graph (ticket names, arrows for blockers). This is the only technical detail in the page and gives the user a chance to reshape the plan before any work starts.

The page should be scannable top-to-bottom in under two minutes. If a section needs more than 5–6 lines, it's too detailed — condense it. The user can always open the spec file for full details.

</html-review-content>

Tell the user the file path. **Wait for their approval before continuing.** If they want changes, update the spec file, revise the draft breakdown, and regenerate the review page. Iterate until they approve both the plan and the breakdown.

---

## Phase 3 — Tickets

The user has approved the spec and the proposed breakdown. Formalize it: write one file per ticket to **`/opt/moatless/workspace/plans/<feature-slug>/<NN>-<slug>.md`**, numbered from `01` in dependency order.

<ticket-template>
# <NN> — <Ticket title>

## What to build

End-to-end behaviour this ticket makes work, from the user's perspective.

## Spec reference

Feature: <feature-slug> — `/opt/moatless/workspace/specs/<feature-slug>.md`

## Blocked by

Ticket numbers/titles that must complete before this one can start, or "None — can start immediately".

## Acceptance criteria

- [ ] Criterion 1
- [ ] Criterion 2

## Status

ready-for-agent
</ticket-template>

Present the breakdown as a numbered list (title / blocked-by / what it delivers). Ask whether granularity is right and whether edges are correct. Iterate until approved.

---

## Phase 4 — Spawn

Find the current repo ID:

```bash
moat repos list --json | jq '.[] | {id, name, remoteUrl}'
```

Identify which tickets have no blockers (or whose blockers are already complete). Spawn **all unblocked tickets in parallel** — one `moat tasks create` per ticket. Capture each child task ID.

For each unblocked ticket:

```bash
CHILD_ID=$(moat tasks create \
  --repo <repo-id> \
  --parent $TASK_ID \
  --name "Ticket <NN>: <slug>" \
  --message "$(cat /opt/moatless/workspace/plans/<slug>/<NN>-<file>.md)

---
ORCHESTRATOR_TASK_ID: $TASK_ID

Use /implement-ticket to implement this work.
" --json | jq -r '.id')
echo "Spawned $CHILD_ID for ticket <NN>"
```

Record a mapping of ticket number → child task ID for monitoring.

---

## Phase 5 — Monitor and Review

Messages from child tasks arrive automatically as system reminders in your conversation — no polling needed. When a child runs `moat send --task $TASK_ID`, it appears inline and you are re-invoked to handle it.

To check overall status at any point:

```bash
moat tasks children --json
```

**When a child reports "PR ready: <url>":**

1. Run `/code-review` on the PR branch vs main:
   - Pin the fixed point as `main` (or the base branch)
   - Identify the spec at `/opt/moatless/workspace/specs/<slug>.md`
   - Run Standards + Spec review in parallel

2. If review finds real issues, send feedback to the child:
   ```bash
   moat send --task <child-id> --text "Code review feedback for your PR:

   <findings>

   Please fix these and push an update, then message me again when ready."
   ```

3. If review passes (or after fixes are applied), mark the ticket complete and check whether any blocked tickets are now fully unblocked. Spawn those tickets immediately.

**When a child reports "Blocked: <description>":**

Send a clarification or resolve the blocker, then let the child continue or close and re-spawn as needed.

Repeat until all tickets are complete and all PRs have passed review.

---

## Phase 6 — Summary

Report to the user:

- All tickets: title, PR URL, review outcome
- Any open items or follow-up work
- Suggested next steps (merging, deployment, closing the spec issue)
