---
on: schedule
name: Agent CLI Upgrade
workspace: moatless-global
agentProfile: claude-code
tags:
  - agent-cli-upgrade
closeAfterSettledDays: 1
schedule:
  # 7 fields: sec min hour day-of-month month day-of-week year. 06:00 UTC, after
  # the 03:00 Nightly E2E has released the cluster.
  cron: "0 0 6 * * * *"
  timezone: UTC
  message: "Check npm for new Claude Code, Codex, and Pi CLI releases, upgrade the sandbox pins, and move the model catalogs to the current models."
---

# Agent CLI upgrade — bump the pins, move the catalogs, prove the agents run

`@anthropic-ai/claude-code`, `@openai/codex`, and `@earendil-works/pi-coding-agent` ship several releases a week,
and new Claude and Codex models arrive between them. The sandbox image bakes one
version of each CLI and the catalog pins each model choice to one model id, so
both drift behind what the product could run. This run closes that gap: read
what npm publishes, bump every copy of the pin, reconcile both model catalogs,
prove both CLIs still drive the agent, and open one PR.

Open a PR only when something moved. Nothing to bump and no catalog change is a
normal result — say so in one line and stop. Do not open an empty PR, and do not
bump anything this prompt does not name.

## 0. Carry on an earlier run's open PR

List this loop's open PRs first:

```bash
moat gh pr list --state open --search 'in:title "chore: bump claude-code" OR in:title "chore: move the claude catalog"'
```

When one is open, do this run on its branch rather than beside it: check the
branch out, `git merge origin/main`, then run every step below. Commit the
run's changes on top and push with a plain `git push`; rewrite the title and
body with `moat gh pr edit` so they describe the branch as it now stands.

Never rebase or force-push that branch: a person may have pushed review fixes
to it, and a rewrite drops them. The repo squash-merges, so the extra commits
cost nothing. When the merge conflicts, report the conflicting files, push
nothing, and stop.

Never stop only because a PR is open, and never open a second one: an
unmerged PR then holds every later release and model back until a person
notices.

## 1. Read what is pinned and what is published

`docker/cli-versions.env` is the source of truth for all three pins.

```bash
grep -E '^(CLAUDE|CODEX|PI)_CLI_VERSION=' docker/cli-versions.env
npm view @anthropic-ai/claude-code version
npm view @openai/codex version
npm view @earendil-works/pi-coding-agent version
```

Take the `latest` dist-tag, which is what `npm view <pkg> version` prints. Do not
take `next`.

When all pins already match, skip to section 3b: a new Claude model needs no
new CLI when the pinned one already reaches its `minCli`.

## 2. Bump every copy of the pin

The Claude version is written in five places and the Codex version in two. A
bump that misses one leaves the image and the test asserting different versions,
and `crates/claude-agent-sdk`'s smoke test fails on the mismatch.

| File | Pin |
|------|-----|
| `docker/cli-versions.env` | all three — the source of truth |
| `docker/Dockerfile.sandbox-base` | all three `ARG` fallbacks |
| `crates/claude-agent-sdk/Dockerfile.test` | `ARG CLAUDE_CLI_VERSION` |
| `crates/claude-agent-sdk/scripts/test-docker.sh` | the `CLAUDE_CLI_VERSION` default, twice |
| `crates/claude-agent-sdk/tests/smoke.rs` | `EXPECTED_CLAUDE_CLI_VERSION` |

Search for the literal old version to confirm you got them all:

```bash
grep -rn "<old-claude-version>\|<old-codex-version>" --include='*' . | grep -v node_modules
```

## 3. Reconcile the Codex model catalog

Skip this when the Codex pin did not move.

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
efforts. Reconcile these places against them:

- `crates/types/src/model.rs` — `CodexModel`: a variant per model in the order
  `model/list` returns them, its `id`, `label`, `price` and `effort_levels`.
  Skip a `hidden` model. Codex fails the whole turn on an effort its model
  rejects, so `effort_levels` must name only what the probe lists for it.
- `crates/core/src/agent_catalog.rs` — `CODEX.models`, the same order, after
  the `DEFAULT_MODEL` entry.
- `DEFAULT_CODEX_MODEL` in `crates/core/src/config.rs` — set to the entry whose
  `isDefault` is true.

Label a model the way the catalog already labels its siblings: `GPT-6 Astra` for
`gpt-6-astra`, spaces rather than the hyphens `displayName` uses. Price a new
model from OpenAI's published rates; when you cannot find them, say so in the
PR body rather than copying a sibling's.

The published Codex list is asserted verbatim in `crates/t3code/src/providers.rs`
by `codex_models_are_published_unchanged` and
`codex_models_keep_the_catalogues_own_names`. Update those tests to the new list
— they are what stops a silent drift, so they move with the catalog, never
around it.

## 3b. Reconcile the Claude model catalog

Run this every time, whether or not a pin moved.

An alias does **not** follow the latest model on its own.
`ClaudeModel::api_id` in `crates/types/src/model.rs` resolves `opus`, `sonnet`,
`haiku` and `fable` to one model id before the CLI sees it, because the proxy
routes on the id. `claude --help` therefore says nothing about what this
catalog runs. The probe compares that table with upstream T3's model manifest,
where each current model lists the bare alias it answers to:

```bash
scripts/claude-model-probe.sh
```

Act on each line of its `== changes` section, `alias: ours -> latest`:

1. **Move the alias only when it can run.** Its `minCli` must not carry
   `(>pin)` against the pin this run ends with, and `routable` must be `yes`.

   When either fails, leave the alias and say which in the PR body. A model the
   proxy does not list fails every turn that picks it. `routable` reads a model
   list, so `yes` is only a hint: the moved id must still answer in section 4's
   Claude transport run.
2. **Set `ClaudeModel::api_id` to `latest` and `label` to the probe's `name`
   verbatim.**

   T3 matches a stored selection against the slug and renders the name, and
   `published_slug` publishes `api_id` as that slug.
3. **Set `ClaudeModel::price` from the pricing row the probe prints.** Use base
   input, 5m cache write, cache read and output, in that order.

   A model with several rows is priced by prompt length. Take the shortest
   prompt's row and say so in the `price` doc.
4. **Keep the old model priced.** Turns that ran on it are still reported
   under its id. Add its family to `EARLIER_CLAUDE` with its own row from the
   pricing page, unless an entry there already prices it right. Keep the list
   most specific first.
5. **Update the tests that spell the ids and names out:**
   `claude_models_are_published_under_t3s_canonical_names` and
   `claude_models_are_named_the_way_t3_names_them` in
   `crates/t3code/src/providers.rs`,
   `a_requested_alias_reaches_the_cli_as_a_model_id` in
   `crates/sandbox/src/agent/options.rs`, and the tests in
   `crates/types/src/model.rs`. Then search for the old id
   (`grep -rn '<old-id>' crates --include='*.rs'`): a test using it as a
   picker slug moves to the new one; a test about a model the catalog does not
   name keeps it.

Compare the `catalog:` price printed beside each pricing row even when no
alias moved, and correct a price that disagrees. A `(no catalog change)` line
with every price matching needs nothing here.

A line reading `alias: no current model in the manifest` means upstream T3
dropped or renamed the model the alias answers to. Move nothing for it, and
name it in the PR body.

A new alias (`claude --help` lists one `ClaudeModel` has no variant for) is a
new variant across `alias`, `api_id`, `label`, `price` and `ALL`, plus an
entry in `crates/core/src/agent_catalog.rs`'s Claude model list.

## 4. Verify

Run the repo's Rust floor first, from the checkout root:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p moatless-types
cargo test -p moatless-t3code
cargo test -p moatless-tasks
cargo test -p moatless-core
cargo test -p moatless-sandbox --lib
```

Then prove the new CLIs actually drive the agent. This is the step a version
bump alone cannot do, and the reason this runs as a loop: install both candidate
CLIs into a temp prefix and point the transport E2E at them.

Install both even when only one moved — the other's version is then the pin
already in `cli-versions.env`, which keeps one command shape for every run.
Which transport gates which CLI is the rule below the block. When section 3b moved
an alias, set `CLAUDE_TEST_MODEL` to the new id, so the run proves the model
answers through the CLI and not only through `curl`.

```bash
npm i --prefix /tmp/cli-probe "@anthropic-ai/claude-code@<new-claude>" "@openai/codex@<new-codex>" --no-audit --no-fund
export CLAUDE_CLI_PATH=/tmp/cli-probe/node_modules/.bin/claude
export CODEX_CLI_PATH=/tmp/cli-probe/node_modules/.bin/codex
unset CODEX_AUTH_JSON CLAUDE_CODE_OAUTH_TOKEN
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

Both CLIs run on the backend's model gateway, the way a pod's agents do: the
harness's `gateway()` reads `BACKEND_API_URL` and the task token in
`/run/moatless/.user_auth_token`. This pod's `ANTHROPIC_API_KEY` and
`OPENAI_API_KEY` may be set but empty; the gateway path does not need them.
Export no further credential, and keep the `unset` in the block: the
backend injects the task user's `CODEX_AUTH_JSON` into this pod, and it or a
`CLAUDE_CODE_OAUTH_TOKEN` sends the CLI to `api.openai.com` or
`api.anthropic.com`, which no deployment uses. Never pass a credential on a
command line, where `moat cmd list` would show it.

A missing credential does not skip the test, so it reads as a broken CLI.
Before you trust a failure, check that `BACKEND_API_URL` is non-empty and the
token file exists. Then ask the gateway for the model with `curl` and the token.
`unknown provider for model` means the gateway has no account for that provider
(see `docs/deferred-changes.md`), not that the CLI broke. A bump verified only by
`clippy` is a bump nobody checked.

**Gate each CLI on its own transport.** A CLI's bump ships only when its
transport passed on the candidate binary. The Claude model moves from section
3b ride on the Claude transport. One CLI failing never holds another CLI that
passed: ship the ones that passed, and leave the one that failed at its old pin.

Leave a held CLI's every pin copy at the old version, and for Codex, leave
section 3's catalog changes out too. Name it in the PR body with the
transport, the failure, and whether it was the CLI or a credential. A CLI
already at `latest` is not on trial: its transport result is context for the
PR body.

When the cause is a credential or a gateway with no account for the provider,
file `moat feedback --category environment` so it gets repaired, unless
`docs/deferred-changes.md` already records it. When the CLI itself broke, a
real regression, quote the error in the run's report.

Open no PR only when every CLI you set out to bump was held and no Claude model
moved. Then report each one's failure.

**Never end this run by asking a question.** Nobody is watching a 06:00 schedule,
so a run that stops for an answer stops until a person finds it. Decide with the
rules above and finish: open the PR, or report the failure and open none.

## 5. Open one PR

One commit, conventional-commit title, header at most 100 characters:

```
chore: bump claude-code to <ver> and codex to <ver>
chore: bump claude-code to <ver> and move <alias> to <model id>
chore: move the claude catalog to <model ids>     # no pin moved
```

Name only the CLIs that moved, and leave a held CLI out of the title. Use no
scope. The code-review loop's `ignored_title_patterns` skips `chore(deps`, and
this PR needs a review before a human merges it.

The body carries what a reviewer cannot get from the diff:

- Each CLI's version range, old → new, or the published version it was held
  at and why.
- Every Codex model added, removed or re-defaulted, and the `model/list` field
  that says so. A changed `DEFAULT_CODEX_MODEL` is the line to lead with — it
  changes what every task running "Default" costs.
- Every Claude alias moved, old id → new id, with its old and new price. An
  alias the probe flagged but this run left, and why: `minCli` or `routable`.
- Every price corrected without a model move, old → new.
- The E2E result per transport: passed, failed, or not run for an explicitly
  named reason. A failure holds only its own CLI (section 4).

Create it with `moat gh pr create`, never bare `gh`.

## What this run does not do

It does not bump `TASK_VERSION`, `RUST_VERSION` or anything else in
`cli-versions.env` — those move with the toolchain, not with the agent CLIs, and
`RUST_VERSION` is pinned to `rust-toolchain.toml` by the `toolchain-drift` CI
job. It does not merge its own PR.

A Pi bump must also pass `tests/api/specs/agent-pi-flow.spec.ts` through the local E2E runner with real credentials for both providers. The suite covers tools, steering, cancellation, forks, recovery, and switching.

A mini-swe-agent upgrade updates `MINI_SWE_VERSION` in `docker/cli-versions.env`
and `docker/Dockerfile.sandbox-base`. Keep the bridge's LiteLLM and MCP pins
compatible. Run the Python bridge contract tests and
`tests/api/specs/agent-mini-swe-flow.spec.ts` through the local E2E runner with
both providers before changing these pins.
