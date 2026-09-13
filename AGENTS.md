# AGENTS.md

Moatless Vibe runs coding agents (Claude Code, Codex) inside on-demand sandboxes
against git repositories. People drive an agent directly, or a Loop runs one on
a schedule. `CONTEXT.md` is the glossary, and it names every term this repo
uses. The workspace holds the TypeScript frontend (`apps/`, `packages/`), the
Rust backend and sandbox (`crates/`), the E2E suite (`tests/`) and the
deployment definitions (`docker/`, `docker-compose/`, `k8s/`).

## Running builds, tests and linters

The `testing-guidelines` skill owns the test entry points these blocks omit and
the Turbo filter trap. `e2e-testing` owns the E2E suite in `tests/`, which
exercises the whole stack: its commands, both profiles, and the sweeper that
clears what a killed run left behind.

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

Run those clippy lines verbatim; add no `-p` line for the crate you touched, and
never run `cargo check`. `crates/codex-sdk` sits outside the workspace and
carries its own build and clippy line in its `Cargo.toml`. Read
`rust-dev-guidelines` before you change a cargo invocation. The sandbox
workspace volume is capped and overflowing it evicts the pod, so check
`df -h /opt/moatless` before a full build and run `task rust:gc` when free space
falls.

### Building, deploying and releasing
```bash
task deploy [TAG=v2.0.0]                                        # also deploy:frontend, status, logs, restart
git tag v0.1.21 && git push origin v0.1.21                      # triggers release.yml
```

## Verifying a change

Mandatory after every change.

- **Rust:** the floor line, plus any `-p` line above naming the crate you
  touched, verbatim. Then `cargo test -p <crate>`.
- **Frontend:** `bun run type:check` and `bun run biome:check`, or
  `bun run post-edit-hook` for a quick loop.
- **Lint ratchet:** `noFloatingPromises`, `noMisusedPromises` and
  `useExhaustiveSwitchCases` are baselined per file in `biome-baseline.json`.
  Fix them in any file you touch, then run `bun run lint:ratchet:update`. A new
  file must be clean, and the baseline only shrinks.

## Writing pragmatic code

- Add no trait or interface with a single implementation. A test mock counts
  only when a test uses it.
- When the code you are changing sits in the wrong module, move it in the same
  change and say so in the PR body, so a reviewer reads the rename apart from
  the edits that follow it.
- Define a verifiable success criterion before you start: a failing test, or a
  reproducible check.

## Writing a commit and a pull request

Write conventional commits: `type(optional-scope): subject`, header 100
characters or fewer. **`commitlint.config.ts` is the authority on which types
and scopes are valid** — this file restates neither list, so settle a case by
asking the linter: `printf 'feat: x\n' | bunx commitlint`.

**Two checks run, over different commits.** `.husky/commit-msg` lints each local
commit; `.github/workflows/pr-title.yml` lints the **pull request title**, which
is what reaches `main`, because this repo squash-merges and GitHub composes the
squash commit from the title where no git hook runs. That title check is
advisory — absent from `main`'s required-status-check list, so a red one blocks
no merge and a reviewer has to notice it.

**Land one commit per pull request.** A branch's internal commits never reach
`main`, so use the PR body to sequence anything a reader needs in order.
Pre-push runs `bun check:pre-push` — Biome and type-check only, no tests — so
run the verification above yourself.

## Writing a comment

A comment carries a fact the code cannot show. Work through this in order:

1. **Default to none — write only when the fact is** an invariant, a "do not do
   X" with what breaks, a non-obvious mechanic, or a unit/ordering the
   signature omits. A public item also gets a one-line doc saying what it is.
2. **Stop at five lines for a comment, eight for a module doc; past that, name
   the kind that needs the length, or cut back.** Unnamed excess gets deleted,
   not argued for. A line counts as what it costs wrapped at 80 columns and a
   blank line as nothing, so neither joining two lines nor deleting the
   paragraph break buys anything.
3. **Delete what no reader needs; do not relocate it** — never into an
   `AGENTS.md`. Text moves to the commit or the pull request only when a
   reviewer of that change needs it.

Past the one-line summary:

- **Do not restate the body**, and put a contract in a structured slot. A trait
  method's, an interface member's or a props field's doc is the whole contract,
  and stays.
- **State a design fact once, on the symbol that owns it, never on a sibling**
  — an enum its variants, a field its default and writers, the nearest
  `AGENTS.md` a cross-cutting rule; elsewhere, link to the owner.
- **End a rationale on what depends on it** — the caller or invariant that
  breaks under the alternative. Name it with `deliberately`/`rather than`, and
  leave the alternative itself in the pull request.
- **Use the code's own names, in backticks**, and name the wrong assumption a
  negative prevents: "No `Activated` row here: `active` was already true" stays,
  "Not the same as `Pending`" goes.
- **Put a trap in a `//` comment on the line it protects**: symptom, cause,
  fix. A precondition binding every caller goes once, on the owning function's
  doc, and each call site links back in one line.
- **Narrate no change.** "Previously", "used to" and "no longer" go to the
  commit or PR; a `legacy`/`deprecated` status word is a design fact stated
  once.

`rust-dev-guidelines`/`frontend-dev-guidelines` hold per-language specifics.

**Enforcement.** `clippy::too_long_first_doc_paragraph` caps a public item's
first paragraph at 200 characters and reads no private item. In `arch-rules`,
`comment-block-size` holds the two ceilings above, `agents-md-budget` this
file's own, and `duplicate-comment-prose` "state a design fact once", both
across siblings and between an `AGENTS.md` and the rustdoc it describes. All
three run on a baseline that only shrinks. Every other rule here is unchecked.

## Finding conventions

- `CONTEXT.md` is the domain glossary. Treat current code and tests as the
  authority and a document as a routing aid. Read `.plans/` only when you are
  asked to implement or review that plan; an unscoped feature idea no plan
  covers goes in `docs/ideas/` (see `docs/ideas/README.md`).
- Put a durable local guideline in the nearest relevant `AGENTS.md`. When you
  edit a path, read the `AGENTS.md` files in that directory and its parents. A
  rule stated in a parent is not restated in a child.
- An `AGENTS.md` holds operational guidance only: what a future agent must do
  and must not do, where the code does not already show it. Give a reason only
  when the rule reads as arbitrary without one, and keep it to a clause. A
  decision, a rejected alternative or a history note goes in the commit or the
  pull request when a reviewer needs it, and nowhere when they do not.
- Every `AGENTS.md` has a committed `CLAUDE.md` symlink beside it, and
  `.claude/skills` is a committed symlink to `.agents/skills`. Nothing creates
  either at runtime, so commit `ln -s AGENTS.md CLAUDE.md` when you add one.
- Record a real problem you are deliberately not fixing in
  `docs/deferred-changes.md`, with an anchor and the condition that makes it
  worth doing. Read that file before you start work in an area.
- `routeTree.gen.ts` is generated, and lint and format exclude it. Edit the
  routes, never the tree.
