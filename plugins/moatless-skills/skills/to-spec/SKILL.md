---
name: to-spec
description: Turn the current conversation into a spec and publish it to the project issue tracker — no interview, just synthesis of what you've already discussed.
disable-model-invocation: true
---

This skill takes the current conversation context and codebase understanding and produces a spec. Do NOT interview the user — just synthesize what you already know.

Issue tracker + triage labels: `docs/agents/issue-tracker.md`.

## Process

1. Explore the repo to understand the current state of the codebase, if you haven't already. Use the project's domain glossary vocabulary throughout the spec, and respect any ADRs in the area you're touching.

2. Sketch out the seams at which you're going to test the feature. Existing seams should be preferred to new ones. Use the highest seam possible. If new seams are needed, propose them at the highest point you can. The fewer seams across the codebase, the better - the ideal number is one.

Check with the user that these seams match their expectations.

3. Run the `/writing-specs` skill for the spec's shape — the fixed heading order, the decision table, and the writing rules. Write the spec to `specs/<slug>.md`, then publish it to the project issue tracker. Apply the `ready-for-agent` triage label - no need for additional triage.

There is no page and no interview here — the agreement already happened in the conversation, so this skill jumps straight to `/writing-specs` step 5: one complete markdown file, written once, for the agent that implements it. Nothing in it is capped.

Three points that bite when synthesizing rather than interviewing:

- **Still write `## Current solution`.** Its usual job is to give the interview a shared reference, and there is no interview here — but the second job survives: it is what a reviewer checks the proposal against, and writing it is what catches the dead path or the second mechanism that the conversation never surfaced.
- **Cover every actor in `## Behaviour`, grouped by actor.** Past 7 in a group, use a table rather than cutting anything. Completeness is the point — an agent reads this file whole.
- **No file paths or code snippets** — they go stale fast. Exception: if a prototype produced a snippet that encodes a decision more precisely than prose can (state machine, reducer, schema, type shape), inline it in the relevant decision and note that it came from a prototype. Trim to the decision-rich parts — not a working demo.
