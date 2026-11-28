---
on: github_pr
name: Full-Repo Code Review
workspace: moatless-global
agentProfile: claude-code
model: opus
closeAfterSettledDays: 1
skills: [pr-review]
# sourceMatcher defaults to this repo's owner/repo; the org's
# GitHub App connection is resolved automatically.
adapterConfig:
  # Approvals, CI results and CI bots' comments carry nothing to review.
  ignored_events: [check_run, check_suite, review_verdict]
  ignored_senders:
    - login: github-actions[bot]
      events: [issue_comment, pull_request_review, pull_request_review_comment]
  ignored_title_patterns:
    - '^chore\(deps'
    - '^deps[:(]'
    - '^build\(deps'
    - '^chore: release'
---

# Moatless — Full-Repo Code Review

Review this pull request with the `pr-review` skill above, which owns the
workflow: modes, silence, findings, threads and approval. This prompt adds
only what is Moatless's: where the rules live, the report's compliance
checklist, and the run record.

The repository is multi-domain: a Rust backend and sandbox, shared Rust crates,
shared TypeScript packages, Helm charts and Docker. Classify each changed file
by the table below, load the guideline its row names, and review each domain by
its own rules. The guidelines live in the checkout, under `.agents/skills/**`
and the root `AGENTS.md`.

## File classification → guideline → what to check

| Path pattern | Domain | Read for conventions | Key checks |
|--------------|--------|----------------------|------------|
| `packages/**/*.ts` | TypeScript packages | root `AGENTS.md` | strict TS (no `any` → use `unknown`); a touched file fixes its `biome-baseline.json` ratchet entries (`noFloatingPromises`, `noMisusedPromises`, `useExhaustiveSwitchCases`) and a new file has none |
| `crates/**/*.rs` (except `crates/cli`) | Rust backend | root `AGENTS.md`; `.agents/skills/rust-dev-guidelines/SKILL.md`, the references it loads for the path, and its rule set `references/review.md` | what `review.md` lists under "Check in the diff", and nothing it leaves to CI; each file holds the responsibility `references/file-roles.md` assigns it, with no pass-through service or layer added to fill a template; utoipa OpenAPI annotations on routes |
| `crates/sandbox/src/**/*.rs` | Rust sandbox | the Rust backend row, plus `rust-dev-guidelines/references/sandbox.md` | plus gRPC / Claude-agent orchestration correctness; graceful shutdown; state-machine transitions |
| `crates/cli/src/**/*.rs` | moat CLI | `.agents/skills/moat-cli/SKILL.md` (+ `references/command-reference.md`) | clap command/flag semantics consistent with the docs; `--json` output; error handling |
| `crates/sandbox-proto/**`, `**/*.proto` | gRPC contract | — | field changes are backward-compatible (no renumbering/repurposing tags); regenerate implications noted |
| `**/*.test.{ts,tsx}`, `**/*.integration.test.ts`, `tests/**`, `**/tests/*.rs` | Tests | `.agents/skills/testing-guidelines/SKILL.md`; `.agents/skills/e2e-testing/SKILL.md` | test behavior, not implementation; never mock the unit whose behaviour the test claims to verify (a mockall DAO port or a stub at an external boundary is fine, per `testing-guidelines/resources/strategic-mocking.md`); tier-appropriate coverage. Rust tests are `cargo test` (`#[cfg(test)] mod tests`, `tests/`, env-gated E2E like `CLAUDE_SDK_E2E=1` / `SANDBOX_E2E=1`) — **not Vitest**. `any` is allowed in TS test files. |
| `k8s/helm/moatless/**` | Helm / K8s | `.agents/skills/k8s-install/references/values-schema.md`; `.agents/skills/sandbox-troubleshooting/SKILL.md` | `values.yaml` ↔ template consistency; pod/container security contexts (backend and T3); sandbox isolation set (networkpolicy incl. Cilium, resourcequota, limitrange, per-task namespace/SA); RBAC least-privilege; **no hardcoded secrets** (use `secrets`/`externalSecrets`); probe / PDB / HPA sanity. Do not ask for a `Chart.yaml` version bump: `ci.yml` sets `version` to `0.0.0-<sha>` on every dev chart publish, and no tagged release exists |
| `docker/**`, `docker-compose/**`, `Dockerfile*` | Docker | `.agents/skills/docker-testing/SKILL.md`; `.agents/skills/docker-install/SKILL.md` | multi-stage build correctness; no secrets baked into layers; pinned base images |
| `Cargo.toml`, `Cargo.lock`, `package.json`, `bun.lock` | Build / deps | `.agents/skills/testing-guidelines/SKILL.md` for Vitest bumps | suspicious or major version bumps; a bump moves its plugin family to the same version (`vitest` with every `@vitest/*`) |
| `crates/core/migrations/**` | Migrations | — | next free number on `main`, no clash; forward-safe; a `CREATE INDEX CONCURRENTLY` file carries `-- no-transaction`. Cost: a backfill or `UPDATE` over a large table runs in bounded batches, with no correlated subquery per row; a statement that takes `ACCESS EXCLUSIVE` sets `lock_timeout`; the whole migration finishes inside the backend's startup probe window, since the backend migrates on start; and the order against the code that reads the column survives a rolling deploy (old pods still serve during it) |
| anything else | General | root `AGENTS.md` | general code quality (below) |

CI runs the checks in root `AGENTS.md` → "Verifying a change", so the reviewer
builds and tests nothing locally.

## General Code Quality

Apply to every changed file, on top of the domain rules.

### Logic & Correctness
- Off-by-one and boundary errors; inverted conditions; missing edge cases (empty
  collections, loading/error states).
- **Rust:** `panic!` on runtime paths; `.clone()` masking a borrow problem.
- **TS:** `??` vs `||` on falsy-but-valid values (`0`, `""`, `false`); missing `await`.

### Performance
- N+1 query patterns (SQLx in a loop).
- Blocking calls inside async/reactive code.
- Unnecessary allocation in hot paths.

### Security
- Exposed secrets/credentials in code, config, Helm values, or Docker layers.
- SQL built by string concatenation (require parameterized queries).
- Missing authorization on new endpoints; PII in logs or error responses.

### Docs that must move with the code
- A removed symbol or a changed domain concept: grep `CONTEXT.md` and `docs/` for
  it, and flag prose the diff made false.
- A gap the PR body lists as not covered needs a `docs/deferred-changes.md` entry.
- A shortened comment still carries its trap (code that looks redundant) and the
  qualifiers that scope its claim.

### Readability & Maintainability
- Functions far past their domain's norm — suggest extraction.
- Nesting deeper than ~3 levels — suggest early returns.
- Dead code, unclear names, copy-paste duplication.

## Compliance checklist

On an initial review and an approval pass, add this section where `pr-review`
marks a Loop's sections, listing only the items that fail, each `- [ ]` with a
`file:line` in italics. When none fails, leave the section out: a row of `[x]`
tells the author nothing. The items, by domain:

- **TypeScript:** no `any`; narrow with `unknown`.
- **Rust:** no `panic!` on runtime paths, typed errors on reusable and domain
  APIs; spawned work owned and cancellable; each file holds its assigned
  responsibility (`file-roles.md`); utoipa OpenAPI annotations, parameterized
  SQLx with listed columns.
- **Tests:** behaviour, not implementation, with the unit under test not
  mocked; the right framework and tier (Vitest for TS, `cargo test` for Rust).
- **Helm / Docker:** values↔template consistency, security contexts, sandbox
  isolation intact; no hardcoded secrets.

Write findings and reports to `general:writing-style` when it is loaded. Text
you suggest for a code comment follows the comment rules in root `AGENTS.md`,
and avoids filler such as "load-bearing".

## Recording the Run

After the summary is posted, save what this review found as one record, so the
reviews of every PR can be compared over time. Save it under the PR and head
commit, so a re-review of the same commit replaces its record and a new commit
adds one:

```bash
moat records save review-runs "runs/<number>/<head sha>" "$(cat /tmp/review-run.json)" --scope loop
```

Write `/tmp/review-run.json` with this shape:

```json
{
  "pr": 1234,
  "headSha": "<full head sha>",
  "taskId": "<$TASK_ID>",
  "reviewedAt": "<RFC 3339 now>",
  "author": "<PR author login>",
  "title": "<PR title>",
  "domains": ["rust", "tests"],
  "outcome": "approved",
  "checks": [
    {"id": "rust-file-roles", "status": "fail", "file": "crates/x/src/y.rs", "line": 42,
     "comment": "<one line>"}
  ],
  "resolved": ["<one line per finding from an earlier review now fixed>"]
}
```

- `domains`: of `typescript`, `rust`, `cli`, `proto`, `tests`, `helm`, `docker`,
  `deps`, `migrations`, `general`.
- `outcome`: `approved` when you submitted `APPROVE`, else `commented`.
- `checks`: one entry per checklist item of a touched domain, by the ids below,
  with `status` `pass`, `fail` or `n/a`, and `file`/`line`/`comment` on a `fail`.
  Add an entry with id `general` for each Additional Feedback finding that got an
  inline comment.

| Checklist item | id |
|---|---|
| No `any`; narrow with `unknown` | `ts-no-any` |
| No `panic!` on runtime paths; typed errors | `rust-errors` |
| Spawned work owned and cancellable | `rust-spawned-work` |
| Each file holds its assigned responsibility | `rust-file-roles` |
| utoipa OpenAPI annotations; parameterized SQLx | `rust-openapi-sqlx` |
| Behavior not implementation; unit not mocked | `tests-behavior` |
| Correct framework/tier | `tests-tier` |
| values↔template; security contexts; isolation | `helm-consistency` |
| No hardcoded secrets | `helm-no-secrets` |

A failed save does not undo the review: mention it in the task's last reply and
move on. Earlier runs are `moat records list review-runs --scope loop --prefix
runs/<number>/`.
