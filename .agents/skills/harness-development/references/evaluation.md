# Evaluation scenarios

Use these when evaluating or revising the skill. These are expected outcomes,
not results of a completed evaluation.

## Routing

Run each prompt in a fresh context with only the skill catalog visible. Repeat
each several times and record whether this skill was selected before work.

| Prompt | Expected |
|---|---|
| Add another coding agent to Moatless using its JSON RPC CLI. | Load |
| Make a new agent selectable alongside Claude and Codex. | Load |
| Implement native session recovery for the harness we are adding. | Load |
| Our new CLI integration acknowledges queued prompts before running them. Fix its lifecycle. | Load |
| Plan the backend, sandbox and E2E wiring for a new coding-agent SDK. | Load |
| Add the provider's latest model to the existing Codex model list. | Do not load |
| Upgrade the pinned Pi CLI version and run its existing smoke tests. | Do not load |
| A sandbox is stuck pulling its image. Check kubectl. | Do not load |
| Use moat to create a task with the released Pi profile. | Do not load |
| Add a Slack webhook adapter to Moatless. | Do not load |

## Execution

Compare a fresh-context run with the skill against a run without it, using the
same checkout and native protocol fixture. Inspect transcripts as well as diffs;
record missed invariants, unnecessary work, tokens and elapsed time.

1. Give an agent protocol where prompt responses acknowledge queuing and a
   settled event follows an earlier end event. Require a tool run plus steering.
   Check that input is acknowledged only on consumption, output has one terminal
   outcome, and an unconsumed queued input is not lost.
2. Give a session fixture with tool IDs, compaction and a user entry appended
   before the next Turn-start event. Require restore, a fork at the previous
   Turn, and switch-back. Check native payload preservation and the fork cut.
3. Give an API-key-only integration with inherited subscription credentials,
   an old installed CLI, and a conflicting migration number on main. Check that
   it restricts gateway selection, uses the checkout CLI, and selects a unique
   migration number rather than reporting a successful skipped test run.

Do not claim these evaluations ran merely because the original Pi integration
passed E2E. That integration supplied the examples; it did not test this skill
in a fresh context.
