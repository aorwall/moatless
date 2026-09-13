# AGENTS.md

Moatless Vibe runs coding agents (Claude Code, Codex) inside on-demand sandboxes
against git repositories. People drive an agent directly, or a Loop runs one on a
schedule. `CONTEXT.md` is the glossary, and it names every term this repo uses.

The workspace holds four areas:

- `apps/` and `packages/` hold the TypeScript frontend.
- `crates/` holds the Rust backend and sandbox.
- `tests/` holds the E2E suite.
- `docker/`, `docker-compose/` and `k8s/` hold the deployment definitions.

## Running builds, tests and linters

### Frontend (bun 1.2.19+)
```bash
bun install
bun run dev              # dev server (vite, in apps/frontend)
bun run build
bun run test             # excludes E2E
bun run type:check       # frontend + shared packages
bun run biome:check      # lint/format (biome:fix to auto-fix)
bun run check:turbo      # all checks (fastest full validation)
```

The `testing-guidelines` skill owns the other test entry points and the Turbo
filter trap.

### Rust backend and sandbox
```bash
cargo build -p moatless-backend        # or moatless-sandbox
cargo clippy --workspace --all-targets -- -D warnings        # the floor: every member
cargo clippy -p moatless-core --all-targets --features openapi -- -D warnings
cargo clippy -p moatless-core --all-targets -- -D warnings
cargo clippy -p moatless-ports --all-targets --all-features -- -D warnings
cargo clippy -p moatless-ports --all-targets --no-default-features -- -D warnings
cargo clippy -p moatless-backend --all-targets --features db-tests -- -D warnings
cargo test -p <crate>                       # scope to the crate you changed, never --workspace
task rust:gc                                # reclaim target/debug/incremental
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```

Run those clippy lines verbatim. Do not vary them. Do not add a `-p` line for
the crate you touched. Never run `cargo check`. `crates/codex-sdk` is excluded
from the workspace and carries its own build and clippy line in its
`Cargo.toml`.

In a sandbox the workspace volume is capped, and overflowing it evicts the pod.
Check `df -h /opt/moatless` before the first full build and between commits, and
run `task rust:gc` when free space falls.

Read the `rust-dev-guidelines` skill before you change a cargo invocation.

### Running the E2E suite
The `e2e-testing` skill owns the commands, both profiles, and the sweeper that
clears what a killed run left behind.

### Building, deploying and releasing
```bash
task deploy [TAG=v2.0.0]                                        # also deploy:frontend, status, logs, restart
git tag v0.1.21 && git push origin v0.1.21                      # triggers release.yml
```

## Verifying a change

Verification is mandatory after every change.

- **Rust:** run the floor line, plus any `-p` line above that names the crate you
  touched, verbatim. Then run `cargo test -p <crate>`.
- **Frontend:** run `bun run type:check` and `bun run biome:check`. For a quick
  loop, run `bun run post-edit-hook`.
- **Lint ratchet:** `noFloatingPromises`, `noMisusedPromises` and
  `useExhaustiveSwitchCases` are baselined per file in `biome-baseline.json`.
  When you touch a file with baselined violations, fix them, then run
  `bun run lint:ratchet:update`. A new file must be clean. The baseline only
  shrinks.

## Writing pragmatic code

- Add no trait or interface with a single implementation. A test mock counts
  only when a test uses it.
- When the code you are changing sits in the wrong module, move it there in the
  same change. Say so in the PR body, so a reviewer reads the rename separately
  from the edits that follow it.
- Define a verifiable success criterion before you start: a failing test, or a
  reproducible check.

## Writing a commit and a pull request

Write conventional commits: `type(optional-scope): subject`, with a header of
100 characters or fewer.

**`commitlint.config.ts` is the authority for which types and scopes are
valid.** This file restates neither list. To settle a specific case, ask the
linter:

```bash
printf 'feat: x\n' | bunx commitlint
```

**Two checks run, and they cover different commits.** `.husky/commit-msg` lints
each local commit. `.github/workflows/pr-title.yml` lints the **pull request
title**, which is what reaches `main` — this repo squash-merges, and GitHub
composes the squash commit from the title server-side, where no git hook runs.

**The title check is advisory.** It is absent from `main`'s
required-status-check list, so a red title check does not block a merge and a
reviewer has to notice it.

**Land one commit per pull request.** A branch's internal commits never reach
`main`, so a reviewer sees nothing of a split across them. Use the PR body to
sequence anything a reader needs in order.

Pre-push runs `bun check:pre-push`, which is Biome and type-check only. It runs
no tests, so run the verification commands above yourself.

## Writing a comment

A comment carries a fact the code cannot show. Work through this in order:

1. **Default to none — write only when the fact is** an invariant, a "do not do
   X" with what breaks, a non-obvious mechanic, or a unit/ordering the
   signature omits. A public item also gets a one-line doc saying what it is.
2. **Stop at five lines for a comment, eight for a module doc; past that, name
   the kind that needs the length, or cut back.** Unnamed excess gets deleted,
   not argued for. Five is a summary and four lines of invariant; a line counts
   as what it costs wrapped at 80 columns and a blank line counts as nothing, so
   neither joining two lines into one long one nor deleting the paragraph break
   buys anything.
3. **Delete what no reader needs; do not relocate it.** Text moves to the
   commit or the pull request only when a reviewer of that change needs it, and
   never to an `AGENTS.md` to get a comment under its ceiling.

`rust-dev-guidelines`/`frontend-dev-guidelines` hold per-language specifics.

- **Past the one-line summary, do not restate the body; put a contract in a
  structured slot, not prose.** A trait method's, an interface member's, or a
  props field's doc is the whole contract, and stays.
- **State a design fact once, on the symbol that owns it, never on a sibling**
  — an enum its variants, a field its default and writers, the nearest
  `AGENTS.md` a cross-cutting rule; elsewhere, link to the owner.
- **End a rationale on what depends on it** — the caller or invariant that
  breaks under the alternative — and delete a sentence defending it. Use
  `deliberately`/`rather than` when naming it; a considered alternative goes in
  the pull request, not a comment.
- **Use the code's own names, in backticks** — `kyc_status`, `Pending` — and
  name the wrong assumption a negative statement prevents: "No `Activated` row
  here: `active` was already true" stays, "Not the same as `Pending`" goes.
- **Put a trap in a line comment on the line it protects**: symptom, cause,
  fix. A precondition binding every caller goes once, on the owning function's
  doc; each call site links back in one line.
- **Narrate no change.** Move a sentence about a state no live path reaches
  ("previously", "used to", "no longer") to the commit or PR; a
  `legacy`/`deprecated` status word is a design fact stated once.

**Enforcement.** `clippy::too_long_first_doc_paragraph` caps a public item's
first paragraph at 200 characters, and reads no private item without
`check-private-items`. Three `arch-rules` checks hold the rest:
`comment-block-size`, `agents-md-budget`, and `duplicate-comment-prose`. The
first fails above the five and eight stated above, measuring each line as what
it costs wrapped at 80 columns and a blank line as nothing, on a baseline that
only shrinks. Every other rule here is unchecked.

## Finding conventions

- `CONTEXT.md` is the domain glossary. Treat current code and tests as the
  authority, and treat a document as a routing aid. Read `.plans/` only when you
  are asked to implement or review that specific plan. Put an unscoped feature
  idea that no plan covers in `docs/ideas/` instead; see `docs/ideas/README.md`.
- Put a durable local guideline in the nearest relevant `AGENTS.md`. When you
  edit a path, read the `AGENTS.md` files in that directory and in its parents.
  A rule stated in a parent is not restated in a child.
- When you find a real problem you are deliberately not fixing now, record it in
  `docs/deferred-changes.md`. Give it an anchor and the condition that makes it
  worth doing. Read that file before you start work in an area. It holds the known
  gaps, and its own rules keep it current.
- The repo owns its own agent-config compatibility. Every `AGENTS.md` has a
  committed `CLAUDE.md` symlink beside it, and `.claude/skills` is a committed
  symlink to `.agents/skills`. Nothing creates either at runtime. When you add an
  `AGENTS.md`, commit `ln -s AGENTS.md CLAUDE.md` alongside it.
- An `AGENTS.md` holds operational guidance only: what a future agent must do
  and must not do, where the code does not already show it. Give a reason only
  when the rule reads as arbitrary without one, and keep it to a clause. Never
  move text here to get a comment under its ceiling. A decision, a rejected
  alternative or a history note goes in the commit or the pull request when a
  reviewer needs it, and nowhere when they do not.
- `routeTree.gen.ts` is generated, and lint and format exclude it. Edit the
  routes, never the tree.
