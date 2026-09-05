# moat CLI agent evaluations

Each JSON file in `scenarios/` is an independent evaluation. Add a file to add
a scenario; the Rust runner discovers it without a code change.

- `id`, `prompt`, and `systemInstruction` define what the agent sees.
- `expectations` defines commands that must be invoked. `command` is the first
  two non-global argv tokens; help invocations never count. Optional
  `requiredArgs`, `forbiddenArgs`, and `minInvocations` refine the assertion.
- `mockResponses` is the safety boundary. Every expected command needs a text
  and JSON fixture. Non-help commands without a fixture are rejected and never
  reach a backend; help is delegated to the real CLI built from this checkout.

The default live run evaluates every scenario three times for both
`claude-code` and `codex`, requiring two passes per agent. Limit a run with
`MOAT_CLI_EVAL_SCENARIOS`, `MOAT_CLI_EVAL_AGENTS`,
`MOAT_CLI_EVAL_REPETITIONS`, or `MOAT_CLI_EVAL_MIN_PASSES`.

Each trial's report includes a timestamped `trajectory` of the normalized
harness signals: session/input lifecycle, visible agent content and tool calls,
activity, context usage, terminal outcome, and an explicit timeout marker.
Partial trajectories and the last resolved model survive a timeout. Credential
fields and known credential values are redacted, and individual trajectory
strings are capped at 16 KiB so diagnostic output cannot dominate the artifact.
