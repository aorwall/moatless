# Repository Instruction Authoring

Use this reference when creating or reviewing `AGENTS.md`, `CLAUDE.md`, nested repository instructions, path-scoped rules, or code-review guidance.

## Purpose

Treat repository instruction files as a **small, always-on repository contract**: facts, commands, conventions, architecture boundaries, and recurring corrections an agent should know on most work in that scope.

Do not turn them into general prompt libraries or long workflow manuals.

## What belongs in persistent repository instructions

Good content includes:

- exact essential build/test/lint/typecheck commands when non-obvious;
- architectural invariants and dependency boundaries;
- authoritative source locations;
- project-specific naming/API/data/compatibility/migration conventions;
- validation expectations tied to types of changes;
- generated-code boundaries;
- non-obvious traps and exceptions;
- consequential code-review rules;
- recurring corrections worth preserving.

Avoid:

- generic advice such as “write clean code”;
- generic reasoning rituals;
- long tutorials or multi-step operational procedures;
- exhaustive repository trees or dependency inventories;
- duplicated formatter/linter configuration;
- transient task context;
- security requirements that must be technically enforced;
- contradictory rules across scopes.

## Write rules that can be acted on

Weak:

```markdown
- Test thoroughly.
- Be careful with migrations.
```

Stronger:

```markdown
- After changing `packages/api/`, run `pnpm test api` and `pnpm typecheck`.
- Never edit an applied migration. Add a new forward migration instead.
```

For non-obvious constraints, prefer:

```text
When <condition>:
- Preserve <invariant>.
- Use <safe path>.
- Verify with <observable check>.
- Exception: <real exception>, if one exists.
```

Explain rationale only when it helps the agent generalize or avoid a tempting unsafe alternative.

## Scope guidance close to the code

Root files should contain repository-wide guidance. Put subsystem-specific rules near the subsystem.

Prefer:

```text
repo/
├── AGENTS.md
├── CLAUDE.md
└── services/
    ├── billing/
    │   └── AGENTS.md
    └── search/
        └── AGENTS.md
```

over continually expanding one root file.

When revising a hierarchy:

- identify which instruction files apply to the target path;
- preserve genuinely global rules at the root;
- move narrow rules downward instead of copying them;
- remove stale or contradictory guidance;
- make explicit overrides only where the host supports them.

## `AGENTS.md` conventions

`AGENTS.md` is standard Markdown; there is no required heading schema.

A practical file may include:

- Repository
- Commands
- Architecture
- Implementation conventions
- Validation
- Boundaries
- Known traps
- Code Review Rules

Use only sections that add project-specific signal.

### Codex loading behavior

Codex discovers persistent guidance from global scope and then walks from the project root toward the current working directory. Instructions closer to the working directory appear later and have greater local precedence.

Within a directory, `AGENTS.override.md` can replace `AGENTS.md` for that level.

Codex currently has a configurable combined project-document budget (32 KiB by default). Treat this as a ceiling, not a target. Prefer nested scope and concise files.

## `CLAUDE.md` conventions

Claude Code uses `CLAUDE.md` files as persistent context rather than hard configuration.

Project-level shared instructions can live in:

- `./CLAUDE.md`
- `./.claude/CLAUDE.md`

Personal project-specific instructions can live in `CLAUDE.local.md` and should normally remain uncommitted.

Claude Code loads applicable ancestor instruction files at startup and can load nested instructions when it works in their subdirectories.

Keep each `CLAUDE.md` concise; current Anthropic guidance targets **under about 200 lines** for better adherence.

For larger repositories, prefer nested files or `.claude/rules/` with path scopes.

Imports improve organization but do not reduce context when the imported content loads at startup.

## Portable `AGENTS.md` + `CLAUDE.md`

When multiple coding agents share the repository, prefer one shared source of truth.

A strong default is:

```text
repo/
├── AGENTS.md
└── CLAUDE.md
```

with:

```markdown
@AGENTS.md

## Claude Code

<!-- Add only genuinely Claude-specific guidance here. -->
```

Use `AGENTS.md` for shared repository rules and put only Claude-specific additions beneath the import.

A symlink may also work when the environment supports it and no Claude-specific additions are required, but the import is more portable across operating systems.

Do not duplicate the entire shared file manually; copies drift.

## Code-review rules

Repository review guidance is most valuable for consequential issues a normal linter/test cannot express and reviewers repeatedly explain.

Good candidates:

- compatibility contracts;
- privacy/data boundaries;
- cross-service assumptions;
- persistence/migration requirements;
- externally consumed wire names;
- non-obvious safe alternatives.

For each rule:

1. State the invariant or risk.
2. Scope it to the code it governs.
3. Give a safe path or valid alternative.
4. Include real exceptions if needed.
5. Leave formatting and mechanical checks to CI.

If removing a rule would not materially change behavior or review quality, consider removing it.

Test important rules with:

- one violation case;
- one safe counterexample;
- one unrelated case.

## Default `AGENTS.md` template

```markdown
# AGENTS.md

## Repository

<One or two sentences with non-obvious project context.>

## Commands

- Install: `<command>`
- Targeted tests: `<command>`
- Full tests: `<command>`
- Typecheck: `<command>`
- Lint: `<command>`

## Architecture

- `<important invariant>`
- `<authoritative source location>`
- `<dependency boundary>`
- `<generated-code boundary>`

## Implementation conventions

- `<project-specific convention not mechanically enforced>`

## Validation

When changing `<area/type>`:
- Run `<specific check>`.
- Verify `<observable result>`.

## Boundaries

- `<durable constraint and safe path>`

## Known traps

- `<surprising fact or recurring mistake>`

## Code Review Rules

- `<consequential non-mechanical invariant>`
  Safe path: `<allowed alternative>`.
```

Delete sections that add no signal.

## Default cross-agent `CLAUDE.md`

```markdown
@AGENTS.md

## Claude Code

<!-- Add only Claude-specific guidance that cannot live in AGENTS.md. -->
```

## Cleanup checklist

- Is the rule needed on most tasks in this scope?
- Is it specific enough to verify?
- Does a narrower directory own it?
- Is it really a procedure that belongs in a Skill?
- Is it mechanical enough for CI/test/linter/hook instead?
- Is it already encoded elsewhere?
- Can the agent infer it cheaply from the repository?
- Does it conflict with another loaded instruction?
- Has it produced value in real tasks or reviews?
