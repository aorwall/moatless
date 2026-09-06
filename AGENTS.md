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
filter trap. The `frontend-dev-guidelines` skill owns the component, import and
styling rules.

### Rust backend and sandbox
```bash
cargo build -p moatless-backend        # or moatless-sandbox
cargo clippy --workspace --all-targets -- -D warnings        # the floor: every member
cargo clippy -p moatless-core --all-targets --features openapi -- -D warnings
cargo clippy -p moatless-core --all-targets -- -D warnings
cargo clippy -p moatless-ports --all-targets --all-features -- -D warnings
cargo clippy -p moatless-ports --all-targets --no-default-features -- -D warnings
cargo clippy -p moatless-backend --all-targets --features db-tests -- -D warnings
cd crates/codex-sdk && cargo clippy --all-targets -- -D warnings   # excluded from the workspace
cargo test -p <crate>                       # scope to the crate you changed, never --workspace
task rust:gc                                # reclaim target/debug/incremental
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```

Run those clippy lines verbatim. Do not vary them. Do not add a `-p` line for
the crate you touched. The floor already lints every workspace member at
`--all-targets`, so a narrower run finds nothing new and costs another
incremental unit. Never run `cargo check`: clippy already covers it, and it keys
a separate set of incremental units.

In a sandbox the workspace volume is capped, and overflowing it evicts the pod.
Check `df -h /opt/moatless` before the first full build and between commits, and
run `task rust:gc` when free space falls.

Read the `rust-dev-guidelines` skill before you change a cargo invocation.
`.agents/skills/rust-dev-guidelines/references/build-costs.md` covers why the set
is shaped this way, what each invocation costs, and how the local list relates to
the CI matrix. Its sibling `references/backend.md` covers the db-tests tier and
the three integration targets.

### Running the E2E suite
`tests/api` drives the product through the `moat` CLI, so the same chains run
against either environment. `E2E_PROFILE` picks the environment, `docker` or
`k8s`. A test the profile cannot run skips, and names the missing capability.
The `e2e-testing` skill owns the commands, both profiles, and the sweeper that
clears what a killed run left behind.

### Building, deploying and releasing
```bash
./docker/build-app.sh | build-backend.sh | build-sandbox.sh    # sandbox: --local --only base on Apple Silicon
bun run dev:docker                                              # local Docker Compose
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
valid.** This file does not restate either list. `commitlint.config.ts` extends
`@commitlint/config-conventional` for the types and pins its own `scope-enum`.
A second copy of an enum drifts from the one that runs, and a reader cannot
tell which copy is current. To settle a specific case, ask the linter:

```bash
printf 'feat: x\n' | bunx commitlint
```

**Two checks run, because they cover different commits.** `.husky/commit-msg`
lints each local commit. `.github/workflows/pr-title.yml` lints the **pull
request title**, which is the message that reaches `main`. This repo
squash-merges, and GitHub composes the squash commit from the title
server-side, where no git hook runs. That workflow triggers on `edited` as well
as `opened`, so a retitle re-runs it. A title check that ran only at open time
would attest to a title that no longer ships.

**The title check is advisory.** It is absent from `main`'s
required-status-check list, so a red title check does not block a merge and a
reviewer has to notice it. Making it required is a repository setting.

**Land one commit per pull request.** This repo squash-merges, so a branch's
internal commits never reach `main`. A reviewer sees nothing of a split across
local commits, and each commit costs the time spent keeping it green. Use the
PR body to sequence anything a reader needs in order.

Pre-push runs `bun check:pre-push`, which is Biome and type-check only. It runs
no tests, so a push proves nothing about test state. Run the verification
commands above yourself.

## Writing a comment

A comment carries a fact the code cannot show. Four kinds qualify:

- An invariant.
- A "do not do X", with what breaks if you do.
- A non-obvious mechanic.
- A unit or an ordering the signature does not state.

Default to none. A public item gets a one-line doc that says what it is. The
working ceiling is three lines for a comment and eight for a module doc. Past
that ceiling the content is durable rationale, and it belongs in the nearest
`AGENTS.md`, linked and not copied.

These rules apply to a doc comment and a line comment in every language. The
`rust-dev-guidelines` skill holds the Rust specifics and
`frontend-dev-guidelines` holds the TypeScript specifics.

- **Past the one-line summary, do not restate the body.** When the reader can
  check a sentence against the code below it, delete that sentence. The doc on a
  trait method, an interface member, an abstract method, or a props field is the
  whole contract, and it stays.
- **State a design fact once, on the symbol that owns it.** An enum owns what its
  variants mean. A field owns its default and its writers. A function owns its
  transition and its side effects. A module doc owns the module invariant. The
  nearest `AGENTS.md` owns a cross-cutting rule and its rationale. Everywhere
  else, link to the owner. Two copies diverge on the next change to one of them.
- **Do not copy a doc between siblings.** Sibling types, sibling crates, and
  sibling implementations of one trait share a trait, a base type, or a module.
  Put the shared text there, and link to it.
- **A rationale ends on what depends on it**: the caller, query, migration, test,
  or invariant that breaks under the alternative. Use `deliberately` and
  `rather than` when what follows names the alternative and what breaks. Cite the
  PR when one produced the decision. Record an alternative you considered in the
  PR or in the nearest `AGENTS.md`, never in the comment.
- **Use the code's own names, in backticks**: `kyc_status` and `Pending`, never a
  paraphrase of them.
- **A negative statement names the wrong assumption it prevents.** "No `Activated`
  row is written here: `active` was already true" stays. "Not the same as
  `Pending`" goes.
- **State, do not argue.** Delete a sentence whose only job is to defend the
  sentence above it.
- **Put a trap in a line comment on the line it protects**: symptom, cause, fix.
  A precondition that binds every caller goes once in the owning function's doc,
  and each call site links back in one line. A lint suppression reason states the
  cause only, because the suppression is the fix.
- **Put the contract in the language's structured slot, and do not repeat it in
  prose.** The language skill defines the slot.
- **Narrate no change.** A comment describes the current state. A sentence about
  a state no live code path reaches ("previously", "used to", "no longer", "the
  old X") goes in the commit or the PR. When an old form and a new form coexist,
  the status word (`legacy`, `deprecated`) is a design fact. It goes once, on
  the symbol that carries it. This rule covers Markdown too.

## Finding conventions and durable decisions

- `CONTEXT.md` is the domain glossary. Treat current code and tests as the
  authority, and treat a document as a routing aid. Read `.plans/` only when you
  are asked to implement or review that specific plan. Put an unscoped feature
  idea that no plan covers in `docs/ideas/` instead; see `docs/ideas/README.md`.
- Put a durable local guideline or decision in the nearest relevant `AGENTS.md`.
  When you edit a path, read the `AGENTS.md` files in that directory and in its
  parents.
- When you find a real problem you are deliberately not fixing now, record it in
  `docs/deferred-changes.md`. Give it an anchor and the condition that makes it
  worth doing. Read that file before you start work in an area. It holds the known
  gaps, and its own rules keep it current.
- The repo owns its own agent-config compatibility. Every `AGENTS.md` has a
  committed `CLAUDE.md` symlink beside it, and `.claude/skills` is a committed
  symlink to `.agents/skills`. Nothing creates either at runtime. When you add an
  `AGENTS.md`, commit `ln -s AGENTS.md CLAUDE.md` alongside it.
- Keep local `AGENTS.md` guidance concise and operational. State what a future
  agent must do and must not do. Add the current reason when that reason is not
  obvious. Add no history dump, no plan transcript, and no long writeup of a
  rejected option.
- The language skills are model-invoked: `rust-dev-guidelines` covers
  `crates/`, and `frontend-dev-guidelines` covers `apps/` and `packages/`.
- `routeTree.gen.ts` is generated, and lint and format exclude it. Edit the
  routes, never the tree.
