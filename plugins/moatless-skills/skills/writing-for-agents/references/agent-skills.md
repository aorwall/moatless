# Agent Skill Authoring

Use this reference when creating, revising, reviewing, or evaluating an Agent Skill built around `SKILL.md`.

## Purpose

Create a Skill for a repeatable capability, workflow, checklist, or body of procedural knowledge that should load only for a recognizable class of tasks.

A Skill is preferable when future tasks benefit from loading the procedure on demand rather than keeping it in always-on repository instructions.

## Start from a coherent job

Define:

- **Job:** the single outcome the Skill exists to produce.
- **Triggers:** intents, inputs, file types, or contexts that should activate it.
- **Non-triggers:** adjacent requests that should not activate it.
- **Inputs:** files, arguments, tools, repositories, or systems it expects.
- **Outputs:** artifacts, edits, decisions, reports, commands, or state changes.
- **Constraints:** formats, policies, ordering rules, permissions, or prohibited side effects.
- **Environment:** packages, binaries, network, host features, or filesystem assumptions.
- **Success criteria:** observable conditions separating success from plausible failure.
- **Known failures:** mistakes the Skill specifically needs to prevent.

Keep one Skill focused on one coherent job.

## Bundle structure

Start instruction-only. Add files only when they earn their complexity.

```text
skill-name/
├── SKILL.md
├── scripts/       # optional deterministic utilities/validators
├── references/    # optional on-demand documentation
└── assets/        # optional templates/static resources
```

Use progressive disclosure:

- `SKILL.md` for routing-aware core workflow and definition of done;
- `references/` for detailed material needed only sometimes;
- `scripts/` for deterministic operations/checks;
- `assets/` for templates, schemas, examples, or static inputs.

Keep file references one level deep from `SKILL.md` when practical.

## Required frontmatter

Every portable `SKILL.md` begins with YAML frontmatter containing at least `name` and `description`.

```yaml
---
name: example-skill
description: Performs X for Y. Use when Z or when the user asks for Q.
---
```

### `name`

Follow the open Agent Skills specification:

- 1–64 characters;
- lowercase letters, numbers, and hyphens only;
- no leading/trailing hyphen;
- no consecutive hyphens;
- exactly matches the parent directory name.

### `description`

Treat `description` as routing logic.

It should:

- say **what** the Skill does;
- say **when** it applies;
- include realistic trigger terms;
- distinguish important neighboring non-triggers when helpful;
- remain within the current specification limit;
- use neutral third-person wording for broad compatibility.

Prefer:

```yaml
description: Reviews database migration plans for safety, reversibility, and rollout risk. Use when creating or auditing schema migrations, data backfills, or migration runbooks. Not intended for ordinary query optimization.
```

Avoid:

```yaml
description: Helps with databases.
```

Add optional metadata only when it serves a real host/environment need. Treat experimental or host-specific fields as extensions rather than portable assumptions.

## Write the body for execution

A strong default structure is:

```markdown
# Skill title

[One concise operational outcome.]

## Workflow
1. ...
2. ...
3. ...

## Decision rules
- If ..., then ...

## Validation
- Check ...
- Success means ...

## Resources
- For ..., read `references/...`
- For ..., run `scripts/...`
```

Use only the sections the task needs.

### Instruction-writing rules

- State each important requirement once.
- Prefer direct imperative language.
- Make meaningful inputs and outputs explicit.
- Use ordered steps only when order matters.
- Put constraints near the steps they govern.
- Distinguish requirements from defaults.
- Define semantic stop conditions for open-ended workflows.
- Put approval boundaries at actual risk boundaries.
- Do not teach common concepts a capable model already knows.
- Do not prescribe generic chain-of-thought rituals.
- Do not repeat generic “double-check everything” instructions.
- Do not copy host tool documentation unless the Skill adds a task-specific rule.
- Do not bury mandatory behavior only inside examples.

The open Agent Skills specification recommends progressive disclosure, a main body below roughly 5,000 tokens, and keeping `SKILL.md` under roughly 500 lines. Move detailed material into referenced files when needed.

## Match specificity to fragility

### High freedom

Use principles/heuristics when multiple approaches are valid and context determines the best method.

### Medium freedom

Use structured workflows, decision rules, templates, or pseudocode when a preferred pattern exists but adaptation remains useful.

### Low freedom

Use scripts, exact commands, validators, or fixed schemas when consistency is critical or the operation is fragile.

**Use language for judgment; use code for invariants.**

## Add scripts only when justified

A script should materially improve correctness, repeatability, speed, context efficiency, validation, or handling of fragile operations.

Scripts should:

- accept explicit inputs and produce explicit outputs;
- document dependencies;
- fail with actionable errors;
- avoid hidden environment assumptions;
- minimize permissions and side effects;
- be idempotent/reversible when practical;
- return machine-checkable status when useful;
- avoid executing unreviewed remote code.

Tell the agent whether to **run** the script or **read** it. For utilities and validators, running is normally preferable.

## Define evals before bloating the Skill

At minimum define:

1. positive trigger;
2. negative control;
3. representative execution case.

For production use, include explicit invocation, implicit invocation, contextual/noisy matches, adjacent negatives, edge cases, and past failures. A small 10–20 prompt regression set is often enough to expose routing and execution regressions early.

Evaluate:

- **Outcome** — did the task complete correctly?
- **Process/routing** — did the right Skill activate and use required steps/tools?
- **Contract/style** — did output meet required structure/conventions?
- **Efficiency** — did the agent avoid needless loops, commands, and context?

Prefer deterministic graders for objective conditions: tests, schemas, exit codes, file/state checks, or invariants. Use rubric/model grading only for genuinely judgment-based qualities.

## Diagnose failures by type

- **Routing failure** → edit `name`, `description`, or Skill boundaries.
- **Missing knowledge** → add the smallest relevant instruction/reference.
- **Weak procedure** → tighten steps or decision rules.
- **Instruction non-compliance** → remove conflicts/redundancy or move invariants into validation/code.
- **Tool/environment failure** → repair tooling or compatibility assumptions.
- **Verification failure** → improve definition of done/validators.
- **Overconstraint** → remove unnecessary prescription.
- **Context bloat** → delete, split, or progressively disclose content.

Change one meaningful thing at a time when practical and rerun the affected evals.

## Default Skill template

```markdown
---
name: skill-name
description: [What it does]. Use when [specific triggers/context]. Not intended for [important adjacent case, if needed].
---

# Skill title

[One concise sentence describing the operational outcome.]

## Workflow
1. [First meaningful action.]
2. [Next action.]
3. [Validation/completion action.]

## Decision rules
- If [condition], [action].

## Validation
- [Objective check or rubric.]
- Success means [observable definition of done].

## Resources
- For [condition], read `references/[file].md`.
- For [deterministic operation], run `scripts/[script]`.
```

Delete anything unnecessary. A good Skill may need only metadata, a short workflow, and a definition of done.

## Skill checklist

- One coherent job.
- Valid `name` matching the directory.
- `description` says what and when.
- Important adjacent non-triggers are distinguishable.
- Body is concise and operational.
- Detailed content is progressively disclosed.
- Scripts exist only when justified.
- Success is observable.
- Positive and negative routing tests exist.
- Representative execution evals exist.
- Permissions and side effects are no broader than needed.
- Environment assumptions are documented where material.
