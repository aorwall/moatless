# Arch-rules: adopt Bun's tooling patterns

> **Status:** approved; items 1, 2 and 4 shipped, item 3 is this PR, and item 5
> ran as a spike that concluded in no rule
> **Updated:** 2026-09-14
> **Vocabulary:** CONTEXT.md

`oven-sh/bun` became a Rust workspace in May 2026 and arrived at the same
problem this repo has: holding architectural decisions that `cargo` and
`rustc` cannot express, in a tree where most commits are written by agents.
It solves that with four layers. This plan takes the three that improve on
what `tooling/arch-rules` does today, adds the one capability it lacks, and
records why the fourth is refused.

What matters for the work below: Bun reaches for `clippy.toml` first and a
custom tool last, it never baselines a style rule, and it writes down why a
lint is off.

## Anchors

| Anchor | What is there |
|---|---|
| `clippy.toml:13` | `disallowed-methods = [` — two entries, `ScopeContext::system` and `SystemTime::now` |
| `clippy.toml:8` | The comment recording that an unresolvable path fails silently |
| `tooling/arch-rules/src/runner.rs:16` | `Rule::check` is `fn(&Workspace) -> Result<Vec<Violation>>` — no changed-line set anywhere in the interface |
| `tooling/arch-rules/src/runner.rs:35` | `rules!(` — the single registration point, 27 rules |
| `tooling/arch-rules/src/comment_block_size.rs:17` | `RULE`; `DOC_CEILING = 5` at `:20`, `MODULE_DOC_CEILING = 8` at `:23` |
| `tooling/arch-rules/src/comment_block_size.rs:118` | `blocks_in_file` — per-file, and the span it returns is what `--since` intersects |
| `tooling/arch-rules/src/public_api.rs:78` | `exports` — driven by `ws.sources(member)`, so workspace members only |
| `tooling/arch-rules/src/public_api.rs:543` | `inherent_signature` — collects the *type names* in a `pub` method's signature, never the method name |
| `tooling/arch-rules/tests/architecture.rs:102` | `the_baseline_holds_no_fixed_violations` — the ratchet |
| `tooling/arch-rules/tests/architecture.rs:121` | `every_registered_rule_is_enforced` — every registered rule must have no unbaselined violation |
| `tooling/arch-rules/arch-baseline.json` | 116 entries. Item 2 dropped the 277 `comment-block-size` ones |
| `.github/workflows/ci.yml:165` | The `architecture` job, `timeout-minutes: 10` |
| `.github/workflows/ci.yml:184` | The `architecture` job's second step — item 2's comment-block gate |
| `rust-toolchain.toml` | `channel = "1.97.0"`, coupled to the sandbox image by `toolchain-drift` |

<!-- plan-assert: contains clippy.toml disallowed-methods = [ -->
<!-- plan-assert: contains tooling/arch-rules/src/comment_block_size.rs pub const RULE: &str = "comment-block-size"; -->
<!-- plan-assert: contains tooling/arch-rules/tests/architecture.rs fn the_baseline_holds_no_fixed_violations -->
<!-- plan-assert: contains tooling/arch-rules/tests/architecture.rs fn every_registered_rule_is_enforced -->
<!-- plan-assert: contains tooling/arch-rules/src/public_api.rs fn inherent_signature -->
<!-- plan-assert: contains rust-toolchain.toml channel = "1.97.0" -->

Two numbers below gated a decision: 277 carried item 2's whole case, and 20 was
why `SystemTime::now` was item 4's first candidate. Both have landed, so each
assertion now pins what the change left behind — a baseline holding no
`comment-block-size` entry, and the four `SystemTime::now` calls the fence
exempts. The rest of item 4's table sizes nothing and is left unasserted,
because those counts move with unrelated work.

<!-- plan-assert: absent tooling/arch-rules/arch-baseline.json comment-block-size:: -->
<!-- plan-assert: contains .github/workflows/ci.yml --comments --since -->
<!-- plan-assert: count 4 crates SystemTime::now -->

## Summary

| | Item | Why | Size |
|---|---|---|---|
| 1 | Resolve-check workspace-internal `disallowed-methods` paths | A renamed path disarms the fence silently; the repo documents this and cannot catch it | medium |
| 2 | Move `comment-block-size` out of `RULES` to a diff-scoped check, delete its 277 baseline entries | 70% of the baseline is a style rule nobody will spend down | medium |
| 3 | Record why a rule is not registered | No trace today of a rule considered and refused | small |
| 4 | Widen the clippy fence, one candidate per change | Bun's primary boundary mechanism; ours holds one entry | medium, ongoing |
| 5 | Spike `hawk` for dead `pub` items | Cross-crate reachability nothing here computes | spike |
| — | **Not** adopting dylint | Requires nightly; we pin stable `1.97.0` | — |

Items 1, 2, 3 and 5 are independent. Item 4 is not blocked by item 1 — see
the note at the end of item 1 for why the obvious dependency does not hold.

## 1. Resolve-check workspace-internal `disallowed-methods` paths

`clippy.toml:13` states the failure in prose:

> An unresolvable `disallowed-methods` path fails silently — clippy reports no
> error — so moving or renaming `ScopeContext::system` disarms this fence with
> nothing to catch it.

The stated mitigation is that a person adds a temporary unannotated call and
checks clippy refuses it. Nothing enforces that.

**The failure mode is specific to paths we own.** We rename
`ScopeContext::system`, we update every call site, the workspace compiles, and
the fence is off with nothing red. An external path cannot fail that way from
our edits. That asymmetry sets this item's scope: it guards
workspace-internal entries only.

**The mechanism costs more than reusing `exports`.** `Export` carries `name`,
`file`, `kind`, `path` and `signature`, and `inherent_signature`
(`public_api.rs:464`) collects the *type names* appearing in a `pub` inherent
method's signature — never the method names. So `exports` reaches the type
`ScopeContext` and stops. A check built on it today would confirm the type is
still exported and stay green through exactly the rename it exists to catch.

Reaching the trailing segment needs `public_api` to expose inherent-impl and
trait item names. That is an additive field on `Export` over the walk that
already visits those impls, and it is part of this item's cost rather than a
detail — it is why the size is medium and not small.

**Acceptance**

- `public_api::Export` exposes associated item names; existing rules that read
  `signature` are unaffected.
- New module under `tooling/arch-rules/src/`, registered in `runner::RULES`,
  with one test in `tests/architecture.rs`.
- Fixture tests drive the decision function with hand-written input, per the
  crate's own "split the judgement from the workspace read" rule.
- Renaming the associated fn `system` in a scratch commit makes it fail.
- An entry naming a non-workspace path is skipped, not reported — with the
  skip visible in the report rather than silent.
- Reports zero on `main` and adds no baseline entries.

**This does not cover item 4.** Every candidate there names a `std` or
third-party path, and `exports` reads `ws.sources(member)` — workspace members
only. There is nothing to resolve them against. That is acceptable rather than
a gap, because an external path cannot go stale under our edits; it goes stale
only when upstream renames, which breaks compilation at the call sites that
still use it. If item 4 ever fences a workspace-internal path, this rule
covers it automatically.

## 2. Move `comment-block-size` to a diff-scoped check

277 of 393 baseline entries are `comment-block-size`, frozen in the day the
rule landed. The count has not moved, and a baseline that size has stopped
being a to-do list.

Bun enforces the same policy without the debt. `.github/workflows/comment-cop.yml`
reads the pull-request diff for added multi-line comment groups, skips groups
containing a `SAFETY:` marker, and posts a review comment. It only ever looks
at what the change added.

**Take it out of `runner::RULES` rather than teaching the rule about diffs.**

`Rule::check` is `fn(&Workspace) -> Result<Vec<Violation>>` and the suite
enters through `run_all(&ws)`. No changed-line set exists in that interface,
so diff-scoping in place means plumbing one through `Workspace` or through the
signature all 27 rules share. The check itself does not need that: it is
per-file and already line-addressed.

So ship it as its own entry point — `arch-report --comments`, taking an
optional `--since <ref>` — and call that from a CI step shaped like the
existing grep-only `plan-assertions` job. The other 26 rules keep their
signature and their baseline untouched.

**Where enforcement ends up, stated plainly.** Today the ceiling is held by
the `architecture` job through `every_registered_rule_is_enforced`
(`architecture.rs:126`), which asserts every registered rule has no
unbaselined violation. After this lands `comment-block-size` is not a
registered rule, so that job stops holding the ceiling and the new CI step is
the only thing that does. `ARCH_STRICT=1` no longer covers it either — worth
knowing, since `tooling/arch-rules/AGENTS.md:32` names that flag as the
eventual end state for the whole set.

**The trade is larger for us than for Bun.** Their comment-cop runs only on
`claude`-labelled pull requests and posts a comment; a failure to post is a
warning, not a red check. A block that slips past an advisory bot costs an
unposted suggestion. A block that slips past the hard gate proposed here is a
hole in the only enforcement there is. A reviewer should decide whether this
lands as a gate or, like Bun's, as a comment.

**Acceptance**

- `arch-baseline.json` drops to 116 entries, none of them `comment-block-size`.
- `cargo test -p moatless-arch-rules` passes on `main` unchanged, and both
  summary lines at `arch_report/report.rs:70`-`:71` read 116: "violation(s)
  total" rather than 393, "in the baseline" rather than 393. Both 393s are
  measured — `arch-report` on `3c142cb6`, where the two lines agree because no
  two violations share a `Violation::id`. No `count` marker or CI job pins
  them, so re-measure rather than trust them if the tree has moved.
- `tests/architecture.rs:91` and the `comment_block_size` import at `:17` are
  deleted rather than left to pass against an empty set. `lib.rs:15` keeps
  `pub mod comment_block_size;`, which the new entry point still needs.
- `arch-report --comments` with no `--since` still lists all 277, so the rule
  stays usable as a cleanup list.
- A scratch commit adding a six-line `///` block fails the CI step.
- A scratch commit touching a file that already holds a long block passes.
- `tooling/arch-rules/AGENTS.md` loses the "baseline that block instead"
  instruction at `:70`, which stops being true.
- `docs/deferred-changes.md:377` is rewritten or dropped. It sizes its fix in
  baseline entries added, and after this there is no baseline to add to.

**Item 2 is done, and it landed as a gate.** The prose above leaves the
gate-or-comment call to a reviewer; the Acceptance list settles it by requiring
a scratch six-line block to *fail* the step, so a gate is what shipped. Flipping
it to advisory is one `continue-on-error: true` line on that step. It lives as a
second step in the `architecture` job rather than a job of its own, because that
job already compiles the crate.

## 3. Record why a rule is not registered

Bun's `dylint.toml` carries a `disabled` list where each entry states the
reason the repo declined it — `unread_error_variant` is off because "Variants
only ever tested through `!=` / `_`; checked, none were bugs", and
`wildcard_over_own_enum` because it is "a per-site style call, ~500 today."

Here a rule is either in `runner::RULES` or it does not exist. The reasoning
behind one that was considered and dropped is nowhere, so the next agent
proposes it again.

**Add a `## Rules considered and declined` section to
`tooling/arch-rules/AGENTS.md`**, one line each: the rule, and what made it
not worth the volume.

**Why this belongs in an `AGENTS.md` at all.** Root `AGENTS.md` says a
decision or a rejected alternative goes in the commit or the pull request, and
never gets relocated into an `AGENTS.md`. This clears that bar because the
entry is not a history note — it is "do not propose X", which is operational
guidance to the next agent, and re-proposal is the exact failure it prevents.
Each line states the instruction, not the deliberation.

**Entry condition.** `tooling/arch-rules/AGENTS.md` is 105 lines with the two
seed entries in place, against `agents-md-budget`'s ceiling of 120, and a
section like this only grows. When it crowds the budget, the oldest entries get
cut rather than the budget raised.

**Seed it with what items 2 and 5 conclude.** Not with the
`generic-module-name` `task/models.rs` case at `AGENTS.md:62` — that is an
exemption carved inside a registered rule, not a rule that was refused, and
filing it here would blur what the section is for.

## 4. Widen the clippy fence, one candidate per change

Bun holds 71 `disallowed-methods` entries, plus 10 `disallowed-types` and 5
`disallowed-macros`, each with a `reason` naming the replacement. All of
`std::fs` routes to `bun_sys`, `std::env::var` to `bun_core::env_var`,
`std::thread::spawn` to `bun_threading::spawn_named`, and every `str` search
method to a SIMD implementation. That is an architectural boundary held in
config, with no custom tool and no extra CI pass, since clippy already runs.
We have one entry and use neither of the other two tables.

Current volume in `crates/`, measured to size the work rather than to propose
all of it:

| Candidate | Lines | Files |
|---|---|---|
| `Utc::now` | 404 | 163 |
| `tokio::spawn` | 194 | 84 |
| `Uuid::new_v4` | 148 | 82 |
| `std::env::var` | 131 | 52 |
| `SystemTime::now` | 20 | 14 |

None is a quick win, and no big-bang fence list should land. Each candidate is
its own change: decide the replacement, migrate the call sites, then add the
entry. `SystemTime::now` at 20 is the smallest first test of the process.

**`SystemTime::now` is done.** Every wall-clock read moved to
`chrono::Utc::now()`, and `clippy.toml` carries the entry. Four sites remain,
all in `crates/s3/src/presign.rs`, where the AWS SDK requires a `SystemTime`
value rather than a moment; each carries an annotated allow. The table above is
the measurement that sized the work, and is not re-derived.

Item 1 does not guard these, and does not need to — see the note closing that
item.

## 5. Spike `hawk` for dead `pub` items

[`astral-sh/hawk`](https://github.com/astral-sh/hawk) finds `pub` items no
shipped root reaches, and over-visible declarations it can downgrade with
`--fix`. Nothing here computes that. `single-implementation-trait` builds a
call graph for a different question — whether a trait has a second
implementation or an exercised double — and `floor-type-admission` counts
consumer crates. Neither answers "is this `pub` item reachable from a binary
at all."

**It carries the constraint this plan refuses dylint for.** hawk links
`rustc_private` and its README says it "must run with the exact Rust toolchain
it was built against", invoked as `cargo +1.98.1 hawk check`. We pin `1.97.0`.
Its README also calls the project experimental and "not intended for public
consumption."

That is survivable for a one-off spike — install 1.98.1 alongside, run it
once, change nothing in the repo. It is not survivable for a CI rule, which
would hit the same wall as dylint. Our binaries are ordinary `[[bin]]`
targets, so none of the staticlib scaffolding in `tools/hawk/README.md` on the
Bun side applies.

**Exit criteria, settled before the spike rather than after.** The reachable
outcomes are a periodic manual sweep or nothing, unless the repo decides to
carry a second pinned toolchain in CI — which is a larger decision than dead
code justifies and should be taken on its own merits, not as a side effect.
Record the outcome under item 3 if it is nothing.

**The spike ran, and the outcome is nothing.** Hawk produced 3246 findings, of
which 158 are `dead_public`. 122 of those are unreachable from the production
binaries and 17 from the workspace tests, but only 8 from both — every one in
prost-generated `sandbox.v1.rs`. No hand-written item is dead by hawk's own
two-root analysis, so not even a periodic sweep is worth the setup. Item 3
carries the line that stops the next agent proposing it.

## Not adopting: dylint

Bun's custom lints run under [dylint](https://github.com/trailofbits/dylint),
which links against `rustc` internals and therefore requires the nightly
toolchain its lints were built against. Bun pins `nightly-2026-07-20` and can
absorb that.

This repo pins `channel = "1.97.0"`, CI runs the `rust:1.97-bookworm` image,
and the `toolchain-drift` job couples that channel to the sandbox base image.
Adopting dylint means moving the whole workspace to nightly, which is a far
larger decision than the lints justify.

The second reason is that our baseline key is already better. Bun stores
`"derived_field:src/css/properties/margin_padding.rs" = 105`, a count per lint
per file: it churns on rename, and a count that falls to zero fails nothing.
`Violation::key` excludes line numbers so it survives code movement, and
`the_baseline_holds_no_fixed_violations` forces the count down. Migrating 27
rules from `syn` to HIR would buy type resolution most of them do not use, and
would cost the ratchet.

## Order

All five are independent; none blocks another.

The one that changes day-to-day work most is 2: it takes the report from 393
violations to 116, and every one of those is architecture rather than prose.
It is also the one with a decision in it — whether the replacement gates or
comments — so it should be taken deliberately rather than first.
