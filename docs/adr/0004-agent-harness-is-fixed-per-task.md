# Agent harness is fixed per Task

A Task uses one Agent harness for its entire lifetime; changing from Claude Code
to Codex, or to another harness, requires a new Task. A Session belongs to its
harness, so allowing per-Turn harness changes would mix incompatible resume and
execution semantics into one conversation and force harness knowledge into the
Task lifecycle module.
