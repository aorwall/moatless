---
on: schedule
name: Agent CLI Upgrade
agentProfile: claude-code
tags:
  - agent-cli-upgrade
inactiveTtlDays: 1
schedule:
  # 7 fields: sec min hour day-of-month month day-of-week year. 06:00 UTC, after
  # the 03:00 Nightly E2E has released the cluster.
  cron: "0 0 6 * * * *"
  timezone: UTC
  message: "Check npm for new Claude Code and Codex CLI releases and upgrade the sandbox pins."
---

# Agent CLI upgrade — bump the pins, prove the new CLIs still drive the agent

`@anthropic-ai/claude-code` and `@openai/codex` ship several releases a week.
The sandbox image bakes one version of each, so the images drift behind the CLIs
the product is written against. This run closes that gap: read what npm
publishes, bump every copy of the pin, reconcile the Codex model catalog against
the new CLI, prove both CLIs still drive the agent, and open one PR.

Open a PR only when something moved. Nothing to bump is a normal result — say so
in one line and stop. Do not open an empty PR, and do not bump anything this
prompt does not name.

## 1. Read what is pinned and what is published

`docker/cli-versions.env` is the source of truth for both pins.

```bash
grep -E '^(CLAUDE|CODEX)_CLI_VERSION=' docker/cli-versions.env
npm view @anthropic-ai/claude-code version
npm view @openai/codex version
```

Take the `latest` dist-tag, which is what `npm view <pkg> version` prints. Do not
take `next`.

When both pins already match, stop here.

## 2. Bump every copy of the pin

The Claude version is written in five places and the Codex version in two. A
bump that misses one leaves the image and the test asserting different versions,
and `crates/claude-agent-sdk`'s smoke test fails on the mismatch.

| File | Pin |
|------|-----|
| `docker/cli-versions.env` | both — the source of truth |
| `docker/Dockerfile.sandbox-base` | both `ARG` fallbacks |
| `crates/claude-agent-sdk/Dockerfile.test` | `ARG CLAUDE_CLI_VERSION` |
| `crates/claude-agent-sdk/scripts/test-docker.sh` | the `CLAUDE_CLI_VERSION` default, twice |
| `crates/claude-agent-sdk/tests/smoke.rs` | `EXPECTED_CLAUDE_CLI_VERSION` |

Search for the literal old version to confirm you got them all:

```bash
grep -rn "<old-claude-version>\|<old-codex-version>" --include='*' . | grep -v node_modules
```

## 3. Reconcile the Codex model catalog

The Codex model list is baked into each CLI build, so a Codex bump can add,
remove or re-default a model. Ask the new CLI directly rather than guessing —
`model/list` over the app-server is the authority.

```bash
scripts/codex-model-probe.sh <old-version> <new-version>
```

That prints both catalogs and the diff between them, and the diff is what you
reconcile: it names exactly the models added, removed and re-defaulted, so a
run where nothing moved reports `(no catalog change)` and needs no further
reading.

**Do not probe with this pod's own `CODEX_HOME`.** The script clears it per
probe because the pod's config routes Codex through the Moatless proxy, and
`model/list` then answers with the proxy's catalog — Claude model ids, and the
proxy's own default marked `isDefault`. Those ids are what would land in the tables below,
and no Codex task can run one.

Each line is `id`, `displayName`, `hidden`, `isDefault` and the reasoning
efforts. Reconcile four places against them:

- `crates/tasks/src/agent/catalog.rs` — `CODEX.models`, in the order `model/list`
  returns them, after the `"default"` entry. Skip a `hidden` model.
- `crates/t3code/src/agent/catalog.rs` — the same list, as `UiRpcModelEntry`.
- `crates/t3code/src/providers.rs` — `CODEX_EFFORT_LEVELS` names only the models
  that accept more than `CODEX_BASE_EFFORT_LEVELS`. Codex fails the whole turn on
  an effort its model rejects, so a row that publishes one hands out a choice
  that cannot run.
- `DEFAULT_CODEX_MODEL` in both `crates/core/src/config.rs` and
  `crates/sandbox/src/agent/codex_app_server/session.rs` — set to the entry whose
  `isDefault` is true, and keep the two identical.

Label a model the way the catalog already labels its siblings: `GPT-6 Astra` for
`gpt-6-astra`, spaces rather than the hyphens `displayName` uses.

The published Codex list is asserted verbatim in `providers.rs`'s
`codex_models_are_published_unchanged`. Update that test to the new list — it is
what stops a silent drift, so it moves with the catalog, never around it.

Claude Code needs no equivalent step. Its catalog entries are the aliases
(`opus`, `sonnet`, `haiku`, `fable`), and an alias already resolves to the latest
model behind it. Check `claude --help` for a new alias and add it only if one
appeared.

## 4. Verify

Run the repo's Rust floor first, from the checkout root:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p moatless-t3code
cargo test -p moatless-tasks
cargo test -p moatless-core
```

Then prove the new CLIs actually drive the agent. This is the step a version
bump alone cannot do, and the reason this runs as a loop: install both candidate
CLIs into a temp prefix and point the transport E2E at them.

Install both even when only one moved — the other's version is then the pin
already in `cli-versions.env`, which keeps one command shape for every run.
Which transport gates the PR is the rule below the block.

```bash
npm i --prefix /tmp/cli-probe "@anthropic-ai/claude-code@<new-claude>" "@openai/codex@<new-codex>" --no-audit --no-fund
export CLAUDE_CLI_PATH=/tmp/cli-probe/node_modules/.bin/claude
export CODEX_CLI_PATH=/tmp/cli-probe/node_modules/.bin/codex
CLAUDE_SDK_E2E=1 CODEX_E2E=1 CLAUDE_TEST_MODEL=<claude-model> CODEX_TEST_MODEL=<codex-model> \
  cargo test -p moatless-sandbox --test agent_transport_e2e \
  -- --ignored --nocapture --test-threads=1 --skip tmux
```

Confirm from the output that the candidates were the binaries under test, not
the image's: `Built agent options` prints `cli_path`, and the Codex app-server
logs `user_agent=moatless-sandbox/<codex-version>`. A sandbox restart deletes
`/tmp`, so a re-run after one silently falls back to `/usr/local/bin` and
verifies the pin already in the image.

`--skip tmux` is required: the tmux tests hang the run rather than failing (see
`docs/deferred-changes.md`). They cover the same two CLIs over the TUI
transport, so skipping them leaves that transport unverified — say so in the PR
body rather than implying the whole suite ran.

Both CLIs run on the proxy this pod already points them at. The environment
holds `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY`, `OPENAI_BASE_URL` and
`OPENAI_API_KEY`, and the harness renders the routed Codex `config.toml` from
them. Export no further credential. A `CLAUDE_CODE_OAUTH_TOKEN` or a
`CODEX_AUTH_JSON` you set here sends the CLI to `api.anthropic.com` or
`api.openai.com`, which no deployment uses, and this pod's proxy key fails
there. Never pass a credential on a command line, where `moat cmd list` would
show it. A missing credential does not skip the test: Claude fails the turn and
Codex 401s through five reconnects, so an empty variable reads as a broken CLI.
Echo the four variables as non-empty before you trust a failure. A bump verified
only by `clippy` is a bump nobody checked.

**The gate covers the CLIs this run is bumping, and only those.** When one CLI
is already at `latest`, its transport is not evidence about this change: its
result goes in the PR body as context, and a failure there does not hold the
PR. A failing transport for a CLI you *are* bumping is the result of the run —
report which CLI and which transport, open no PR, and do not work around it by
reverting to the old pin.

That distinction is what a credential failure turns on. A dead credential for
the CLI being bumped leaves the bump unverified, so it blocks. A dead credential
for a CLI that is not moving blocks nothing — say so in the PR body, and file
`moat feedback --category environment` so the sign-in gets repaired.

**Never end this run by asking a question.** Nobody is watching a 06:00 schedule,
so a run that stops for an answer stops until a person finds it. Decide with the
rules above and finish: open the PR, or report the failure and open none.

## 5. Open one PR

One commit, conventional-commit title, header at most 100 characters:

```
chore: bump claude-code to <ver> and codex to <ver>
```

Use no scope. The code-review loop's `ignored_title_patterns` skips
`chore(deps`, and this PR needs a review before a human merges it.

The body carries what a reviewer cannot get from the diff:

- Both version ranges, old → new.
- Every Codex model added, removed or re-defaulted, and the `model/list` field
  that says so. A changed `DEFAULT_CODEX_MODEL` is the line to lead with — it
  changes what every task running "Default" costs.
- The E2E result per transport: passed, failed, or not run for an explicitly
  named reason. When the failure is a credential, section 4 says whether it
  holds the PR: it does for a CLI this run bumps, and it does not for one
  already at `latest`.

Create it with `moat gh pr create`, never bare `gh`.

## What this run does not do

It does not bump `TASK_VERSION`, `RUST_VERSION` or anything else in
`cli-versions.env` — those move with the toolchain, not with the agent CLIs, and
`RUST_VERSION` is pinned to `rust-toolchain.toml` by the `toolchain-drift` CI
job. It does not merge its own PR, and it does not touch a PR from an earlier
run: when one is still open, say so and stop.
