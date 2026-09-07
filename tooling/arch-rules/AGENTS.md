# tooling/arch-rules

```bash
cargo run -p moatless-arch-rules --bin arch-report                      # the to-do list
cargo run -p moatless-arch-rules --bin arch-report -- --json            # stable IDs and locations
cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline # accept what is there now
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

## Reading the workspace and recording an exception

`Workspace` reads Cargo metadata and raw manifests. Its cached source index
resolves declarations, imports, and test-only modules. Keep this crate
independent of the workspace application crates.

Use `package.metadata.arch.allow` for an exact dependency edge, with `rule`,
`dependency` and `reason`. Use `/// arch-allow(rule): reason` for an item. The
annotation checker rejects an unknown rule, an empty reason, and wrong
placement. Baseline existing debt when a new checker lands, and put a
deliberate exception beside its declaration.
