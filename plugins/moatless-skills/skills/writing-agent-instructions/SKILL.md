---
name: writing-agent-instructions
description: Authors, reviews, and refactors persistent repository instructions in AGENTS.md and CLAUDE.md. Use when creating coding-agent guidance, deciding root versus nested scope, consolidating or pruning instructions, resolving conflicts, or sharing repository guidance across Codex and Claude Code. Not for Agent Skills, one-off prompts, or general project documentation.
metadata:
  version: "1.0.0"
---

# Writing Agent Instructions

Write `AGENTS.md` and `CLAUDE.md` as small, durable repository contracts for coding agents.

Preserve only the project-specific facts, constraints, conventions, and validation expectations an agent needs repeatedly and cannot safely infer on its own.

For every candidate instruction, decide:

1. **Eligibility** — should it live in persistent agent context at all?
2. **Scope** — what is the narrowest meaningful repository subtree that needs it?

## Out of scope

Do not use this skill to write:

- Agent Skills or `SKILL.md`;
- one-off task prompts;
- general onboarding or architecture documentation;
- long release, migration, incident, or other occasional procedures;
- security controls that require deterministic enforcement.

If information does not belong in `AGENTS.md` or `CLAUDE.md`, say so instead of forcing it into these files.

## Principles

1. **Persistent instructions are context, not documentation.**
2. **Put each rule at the narrowest meaningful scope that needs it.**
3. **Keep root guidance useful across most repository work.**
4. **Prefer project-specific meaning over generic engineering advice.**
5. **Do not restate facts the agent can cheaply inspect.**
6. **Write observable instructions, not aspirations.**
7. **Keep each meaning in one authoritative place.**
8. **Avoid conflicting parent and child guidance.**
9. **Use prose for guidance; use tooling for hard enforcement.**
10. **Add rules from observed recurring failures; prune stale rules.**

## Workflow

### 1. Inspect before drafting

Ground instructions in repository evidence. Inspect as relevant:

- existing `AGENTS.md`, `AGENTS.override.md`, `CLAUDE.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md`, and nested instruction files;
- manifests, task runners, build/test/lint/typecheck configuration, and CI;
- architecture boundaries and source-of-truth locations;
- generated-code directories;
- contributor docs;
- tests, linters, schemas, hooks, permissions, and other enforcement.

Do not copy repository facts merely because they are easy to discover.

### 2. Collect candidate persistent instructions

Look for:

- non-obvious commands agents repeatedly need;
- architecture or dependency invariants;
- authoritative schema, contract, or configuration locations;
- generated files that must not be edited manually;
- repository-specific conventions not enforced elsewhere;
- required validation for particular changes;
- recurring agent mistakes or corrections;
- surprising compatibility constraints or exceptions.

Treat every candidate as unproven until it passes the eligibility test.

### 3. Apply the eligibility test

Ask:

> **Will an agent repeatedly need this while working in the affected area, and is it difficult or unsafe to infer reliably from the environment?**

Usually include:

```markdown
- `packages/contracts/` is the source of truth for public API schemas.
- Domain packages must not import provider-specific adapters.
- Files under `generated/` are outputs; edit their source definitions instead.
- After changing database migrations, run `pnpm db:test-migrations`.
```

Usually omit:

```markdown
Write clean code.
Follow best practices.
Think step by step.
Be careful.
```

Also omit dependency lists, directory trees, formatter rules already enforced automatically, temporary task requirements, and long occasional procedures.

If a requirement must never be violated, prefer permissions, CI, tests, hooks, schemas, or another deterministic mechanism. Prose may explain the boundary but should not be its only protection when failure is unacceptable.

### 4. Choose repository scope

For each eligible instruction, identify all files or subsystems whose work requires it.

Use this placement rule:

> **Place the instruction at the nearest meaningful common ancestor of the files whose work requires it.**

Example:

```text
repo/
├── AGENTS.md                    # repository-wide guidance
└── packages/
    ├── frontend/
    │   └── AGENTS.md            # frontend-only guidance
    └── database/
        └── AGENTS.md            # database-only guidance
```

A database migration rule belongs under `packages/database/` when unrelated work does not need it.

A rule shared by all frontend packages belongs at `packages/frontend/`, not separately in each package.

A genuinely repository-wide invariant belongs at the root.

### 5. Prefer the nearest **meaningful** scope, not the narrowest possible path

Nested files should correspond to coherent subsystem boundaries.

Create one when:

- the directory represents a real package or subsystem;
- several related instructions share that scope; or
- an important local rule would otherwise consume broad global context.

Avoid needless instruction topology such as:

```text
src/AGENTS.md
src/api/AGENTS.md
src/api/users/AGENTS.md
src/api/users/routes/AGENTS.md
```

unless those directories are independently governed subsystems.

A one-line nested file is usually a reason to reconsider placement, though a high-value boundary can justify one.

### 6. Keep root strict

Root guidance has the widest context cost. Limit it to information useful across most repository work, such as:

- non-obvious repository identity;
- universal commands that are genuinely needed;
- repository-wide architecture invariants;
- source-of-truth locations;
- universal change boundaries;
- global validation expectations;
- a small number of recurring traps.

Do not place subsystem details at root merely to make them easier for humans to find.

### 7. Respect host semantics

`AGENTS.md` and `CLAUDE.md` solve similar problems but do not have identical loading behavior.

#### AGENTS.md

For hosts supporting hierarchical `AGENTS.md` files:

- root guidance should be broad;
- nested guidance should be subtree-specific;
- more-local guidance should specialize work in that subtree;
- inspect existing `AGENTS.override.md` before changing behavior;
- do not duplicate child rules in ancestors.

Verify current host documentation when exact precedence or syntax matters. Do not assume every `AGENTS.md` implementation behaves identically.

#### CLAUDE.md

For Claude Code:

- use project `CLAUDE.md` for persistent project guidance;
- use nested files for guidance relevant to a particular subtree;
- treat parent and nested guidance as additive rather than assuming child files cleanly replace parents;
- avoid contradictory parent and child rules;
- keep personal or machine-local preferences out of shared project files.

Verify current Claude Code documentation when exact loading behavior matters.

### 8. Prefer one shared source in multi-agent repositories

When both `AGENTS.md` and Claude Code need the same guidance, avoid maintaining duplicate copies.

A common structure is:

```text
repo/
├── AGENTS.md
└── CLAUDE.md
```

with `CLAUDE.md` importing shared instructions:

```markdown
@AGENTS.md

## Claude-specific guidance

<!-- Only genuinely Claude-specific instructions belong here. -->
```

Use the same pattern at meaningful nested scopes when appropriate. Keep host-specific behavior separate when the semantics genuinely differ.

### 9. Write operational contracts

Prefer:

```text
When <condition>:
- Do <specific action>.
- Verify with <command or observable result>.
- Exception: <specific exception>, if one exists.
```

Weak:

```markdown
Be careful with database changes.
Test thoroughly.
```

Strong:

```markdown
## Database changes

- Never edit an already-applied migration.
- Add a new migration for production schema changes.
- After changing migrations, run `pnpm db:test-migrations`.
```

Add a short rationale when a surprising constraint needs context for correct generalization.

### 10. Document meaning, not cheap lookups

Usually omit facts like:

```markdown
The frontend code is in `packages/frontend`.
The project uses TypeScript.
```

Prefer facts with semantic value:

```markdown
- `packages/contracts/` defines the public API contract; generated SDK types are downstream artifacts.
- `packages/domain/` must remain independent of framework and provider adapters.
- `tests/compat/` is the compatibility gate for public schema changes.
```

The filesystem can reveal location. Persistent instructions should explain authority, boundaries, invariants, or non-obvious consequences.

### 11. Keep one source of truth per rule

Do not repeat a child instruction at root “for emphasis.”

For each rule, ask:

- What is its actual scope?
- Which file should authoritatively own it?
- Does another file already express the same meaning?
- Would moving it downward reduce irrelevant context without hiding it from affected work?

When the same rule must reach both hosts, prefer a supported import/shared-source mechanism over duplicated prose.

### 12. Resolve conflicts structurally

When parent and child guidance conflict:

1. identify the true scope of each rule;
2. move the narrower rule to the narrowest meaningful subtree;
3. rewrite the parent rule to express only the global invariant;
4. keep a global exception only when that exception itself must stay globally visible.

Prefer scope over chains of “except in X, unless Y.”

### 13. Validate the instruction tree

Review the affected hierarchy, not only the file just edited.

Confirm:

- root guidance is broadly applicable;
- nested files correspond to meaningful subsystem boundaries;
- rules are not duplicated across scopes;
- parent and child files do not conflict;
- commands and referenced paths exist;
- generated/source-of-truth relationships are accurate;
- hard enforcement is not being replaced by prose;
- files remain concise and easy to scan.

For consequential rules, test three cases:

1. **Violation** — the rule should change behavior.
2. **Safe counterexample** — nearby valid behavior should remain allowed.
3. **Unrelated** — work outside the rule's scope should not be distracted by it.

If an instruction produces false positives, narrow its wording or move it closer to the affected subtree before adding more prose.

### 14. Prune before adding

Before adding a line, ask whether an existing line can be replaced, narrowed, moved, or deleted.

Remove instructions that are:

- stale;
- redundant;
- generic;
- cheaply discoverable;
- already mechanically enforced;
- superseded by a narrower file;
- no longer changing agent behavior usefully.

Let recurring failures justify new persistent guidance. Do not anticipate every possible mistake in advance.

## Review checklist

Before delivering changes, confirm:

- [ ] Every retained instruction belongs in persistent repository context.
- [ ] Root content is useful across most repository work.
- [ ] Local rules live at the nearest meaningful scope.
- [ ] Nested files represent coherent subsystem boundaries.
- [ ] No meaning is duplicated merely for emphasis.
- [ ] Parent and child guidance does not conflict.
- [ ] Host-specific behavior is not presented as portable behavior.
- [ ] Shared `AGENTS.md` / `CLAUDE.md` content has one source of truth where practical.
- [ ] Commands and paths were verified.
- [ ] Instructions are project-specific and observable.
- [ ] Hard invariants remain mechanically enforced where appropriate.
- [ ] Stale, inferable, and redundant material was removed.
- [ ] Important rules have violation, safe, and unrelated test cases.

## Definition of done

The work is complete when every retained instruction has:

1. a clear reason to exist in persistent agent context;
2. an intentional repository scope;
3. one authoritative home;
4. no conflict with inherited guidance; and
5. evidence-backed commands, paths, or invariants where applicable.

The resulting instruction tree should be smaller and more precise than putting every useful-looking rule in the root file.
