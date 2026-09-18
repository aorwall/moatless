# tooling/arch-rules

```bash
cargo run -p moatless-arch-rules --bin arch-report                      # the to-do list
cargo run -p moatless-arch-rules --bin arch-report -- --json            # stable IDs and locations
cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline # accept what is there now
cargo run -p moatless-arch-rules --bin arch-report -- --persistence-inventory # rewrite the DAO migration inventory
cargo run -p moatless-arch-rules --bin arch-report -- --comments               # every over-ceiling comment block
cargo run -p moatless-arch-rules --bin arch-report -- --comments --since main  # only the ones this branch added
cargo run -p moatless-arch-rules --bin arch-report -- --comments --findings b.ndjson # what CI comments with
cargo test -p moatless-arch-rules                                       # fails on new violations
ARCH_STRICT=1 cargo test -p moatless-arch-rules                         # fails on any violation
```

**Run this package with `cargo test`, never `cargo nextest run`.** Every rule
test reads one `run_all` through a process-wide `OnceLock`, and nextest's
process-per-test pays that workspace parse once per test.

## How a rule lands

A rule ships before the workspace satisfies it. Put the violations present on
the day it lands into `arch-baseline.json`. The test then fails only on a
violation that is absent from the baseline.

**The baseline only shrinks.** `the_baseline_holds_no_fixed_violations` fails
when an entry no longer occurs, so drop a fixed violation rather than leave it
to shadow the next one. Review every baseline identity a `--update-baseline`
run adds.

`cargo run -p moatless-arch-rules --bin arch-report` names the entries, as
`N baseline entry(s) now fixed`. Run it after a change that could satisfy a rule
without meaning to: `single-implementation-trait` counts a mock only where a
test exercises it, so adding the first test double for a port trait clears that
trait's entry, and `cargo test -p moatless-arch-rules` then fails on a line
naming the baseline rather than your change.

**A rule at zero is already locked there.** The ordinary test filters to
violations absent from the baseline, so a rule with no baseline entries fails on
any occurrence — `dependency-privacy` sits at zero today. Do not add a per-rule
strict switch.

`ARCH_STRICT=1` reads the *whole* baseline as empty, which asserts that nothing
is baselined anywhere. It is global by design. Turn it on once every rule has
reached zero.

## Adding a rule

- Write one module per rule, named for what it asserts, exporting `RULE` and
  `check(&Workspace) -> Result<Vec<Violation>>`. Register it in `runner::RULES`,
  which `runner::run_all` iterates, and add one test to `tests/architecture.rs`
  so a failure names the rule.
- **Split the judgement from the workspace read.** `check` gathers, a private
  function decides, and the fixture tests beside it drive that function with
  hand-written input. A rule whose boundaries can only be exercised by editing
  the real workspace has no way to pin them.
- **`Violation::key` is the identity of the breach, and never the identity of
  the file it was found in.** Two demands pull against each other, and both must
  hold:
  - The key never contains a line number, or the baseline churns whenever code
    moves. Put lines in `locations`, which is printed and not baselined.
  - The key carries everything a change to the breach would change. `Config`
    alone lets a third crate join an accepted collision unnoticed, so the key
    names the participating crates. A module's key names the module rather than
    the file, so `types.rs` and `types/mod.rs` cannot become two slots.
- **A rule about what crates share reads the module tree, and never the
  filesystem.** `pub` in a private module means nothing outside the crate, and a
  release build compiles no `#[cfg(any(test, feature = "test-support"))] mod`.
  Use `public_api::exports`, which walks from the crate root and follows
  re-exports, rather than `Workspace::sources`, for anything that asserts about
  shared vocabulary.
- **Check the rule against the repo before you trust it.** `generic-module-name`
  first flagged every `task/models.rs` beside `task/schemas.rs`. That pairing is
  a deliberate convention: the directory names the domain, and the leaves split
  the entity from the wire shape. A rule that fights a convention is wrong about the
  convention until proven otherwise.

## `comment-block-size` is not a registered rule

It runs from `arch-report --comments`, and `--since <ref>` narrows it to the
blocks a diff added — which is what CI's `architecture` job reports on a pull
request, posting a review comment per block. Nothing fails on one, so cut an
over-ceiling block in a change you are making rather than wait to be asked. It
has no baseline and no `arch-allow`: writing `arch-allow(comment-block-size)`
makes things worse rather than not working, because `annotations.rs` validates
the name against `RULES` and reports an unregistered one as its own violation.

Its two constants are the 5 and 8 root `AGENTS.md` asks for. Do not raise one to
absorb a long block.

## Rules considered and declined

Each entry is an instruction not to propose that rule again, and what made it
not worth the volume. When this section crowds `agents-md-budget`'s ceiling,
cut the oldest entry rather than raise the ceiling.

- **A dead-`pub` sweep, whether through `cargo hawk` or a rule of our own.**
  Hawk finds no hand-written dead item — the 8 it reaches are prost-generated —
  and it needs its own pinned toolchain to run at all.
- **A whole-workspace comment-block check with no baseline.** Hundreds of blocks
  are already over the ceiling, so one that fails on any of them fails on every
  branch. Scope such a check to the lines a diff added.

## Reading the workspace and recording an exception

`Workspace` reads Cargo metadata and raw manifests. Its cached source index
resolves declarations, imports, and test-only modules. Keep this crate
independent of the workspace application crates.

Use `package.metadata.arch.allow` for an exact dependency edge, with `rule`,
`dependency` and `reason`. Use `/// arch-allow(rule): reason` for an item. The
annotation checker rejects an unknown rule, an empty reason, and wrong
placement. Baseline existing debt when a new checker lands, and put a
deliberate exception beside its declaration.
