---
name: orchestrate
description: "End-to-end feature orchestration: grill requirements, write spec, render HTML for review, break into tickets, spawn parallel child tasks, review PRs, loop until done. Use when a user wants to plan and execute a non-trivial feature across many tasks."
disable-model-invocation: true
---

# Orchestrate

Drive a feature from idea to merged PRs: clarify → spec → review → tickets → spawn → monitor → code-review → loop.

This skill spawns child Moatless tasks to implement work in parallel. Each child creates a PR and reports back; you run code review on each and advance blocked tickets as their dependencies land.

Three artifacts are produced along the way, each with a distinct audience:

1. **Spec markdown** — `/opt/moatless/workspace/specs/<feature-slug>.md` — the source of truth; inlined into every child's spawn message.
2. **Ticket markdown files** — `/opt/moatless/workspace/plans/<feature-slug>/<NN>-<slug>.md` — one per ticket; each becomes the body of its child's spawn message.
3. **HTML report** — via `/html-report`, saved under `/opt/moatless/workspace/reports/` — how you communicate the spec and plan to the user for review and approval. Regenerate it whenever the spec or breakdown changes so the shared page always reflects the current plan.

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

In each ticket's Spec reference section, note that the full spec is included in the child's spawn message — children cannot read the orchestrator's files (see Phase 4).

Present the breakdown as a numbered list (title / blocked-by / what it delivers). Ask whether granularity is right and whether edges are correct. Iterate until approved. Once approved, regenerate the HTML report as a combined spec + tickets page (overview, key decisions, dependency graph, per-ticket cards) so the user-facing page matches the final plan.

---

## Phase 4 — Spawn

Find the current repo ID:

```bash
moat repos list --json | jq '.[] | {id, name, remoteUrl}'
```

Identify which tickets have no blockers (or whose blockers are already complete). Spawn **all unblocked tickets in parallel** — one `moat tasks create` per ticket. Capture each child task ID.

**The spawn message must be fully self-contained.** Each child runs in its own sandbox with a fresh checkout of the repository only — it CANNOT read the orchestrator's `/opt/moatless/workspace/specs/`, `plans/`, or `reports/` files. Inline everything the child needs: the full ticket AND the full spec.

For each unblocked ticket:

```bash
CHILD_ID=$(moat tasks create \
  --repo <repo-id> \
  --parent $TASK_ID \
  --name "Ticket <NN>: <slug>" \
  --message "$(cat /opt/moatless/workspace/plans/<slug>/<NN>-<file>.md)

---
# Feature spec (full text — included because you cannot read the orchestrator's files)

$(cat /opt/moatless/workspace/specs/<slug>.md)

---
ORCHESTRATOR_TASK_ID: $TASK_ID
BASE_BRANCH: <branch>   # only for stacked tickets — see below; omit to base on the default branch

Use /implement-ticket to implement this work. Create a PR when done.
" --json | jq -r '.taskId')
echo "Spawned $CHILD_ID for ticket <NN>"
```

(The create response's field is `taskId`, not `id`.)

**Every ticket ships as its own PR** — one ticket, one branch, one PR. Dependent tickets don't wait for their blocker's PR to merge: **stack them**. When ticket B is blocked by ticket A and A's PR has passed review, spawn B with `BASE_BRANCH: <A's PR branch>` — the child branches from it and opens its PR with that base. Record each child's PR branch (from its report-back) so you can stack dependents on it. Stacked PRs merge in dependency order; after a base PR merges, ask the dependent's child to rebase onto the default branch (GitHub retargets the PR base automatically on merge).

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

3. If review passes (or after fixes are applied), mark the ticket complete and check whether any blocked tickets are now fully unblocked. Spawn those tickets immediately, stacked on the reviewed PR's branch (`BASE_BRANCH: <branch>` in the spawn message) — do not wait for the PR to merge.

**When a child reports "Blocked on: <description>":**

Send a clarification or resolve the blocker, then let the child continue or close and re-spawn as needed.

Repeat until all tickets are complete and all PRs have passed review.

---

## Phase 6 — Summary

Report to the user:

- All tickets: title, PR URL, review outcome
- Any open items or follow-up work
- Suggested next steps (merging, deployment, archiving the spec and plan files)
