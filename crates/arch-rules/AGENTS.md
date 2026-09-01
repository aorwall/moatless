# crates/arch-rules

Architecture rules for the workspace. A rule reads the workspace and returns
violations; `arch-report` prints them, `tests/architecture.rs` enforces them.

```bash
cargo run -p moatless-arch-rules --bin arch-report                      # the to-do list
cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline # accept what is there now
cargo test -p moatless-arch-rules                                       # fails on new violations
ARCH_STRICT=1 cargo test -p moatless-arch-rules                         # fails on any violation
```

## How a rule lands

A rule ships before the workspace satisfies it. The violations present the day
it lands go into `arch-baseline.json`, the test fails only on ones that are
not, and `ARCH_STRICT=1` is the switch once a rule reaches zero. A rule that
had to wait for a clean workspace would never land.

**The baseline only shrinks.** `the_baseline_holds_no_fixed_violations` fails
when an entry no longer occurs, so a fixed violation must be dropped rather
than left to shadow the next one. Running `--update-baseline` to absorb a new
violation is the one motion this design exists to make visible in a diff.

## Adding a rule

- One module per rule, named for what it asserts, exporting `RULE` and
  `check(&Workspace)`. Register it in `run_all` and in `RULES`, and add one
  test to `tests/architecture.rs` so a failure names the rule.
- **Split the judgement from the workspace read.** `check` gathers, a private
  function decides, and the fixture tests beside it drive that function with
  hand-written input. A rule whose boundaries can only be exercised by editing
  the real workspace has no way to pin them.
- **`Violation::key` is the identity of the breach, not of the file it was
  found in.** Two demands pull against each other and both must hold:
  - It never contains a line number, or the baseline churns whenever code
    moves. Lines go in `locations`, which is printed and not baselined.
  - It carries everything a change to the breach would change. `Config` alone
    lets a third crate join an accepted collision unnoticed, so the key names
    the participating crates; a module's key names the module rather than the
    file, so `types.rs` and `types/mod.rs` cannot become two slots.
- **A rule about what crates share reads the module tree, not the filesystem.**
  `pub` in a private module means nothing outside the crate, and a
  `#[cfg(any(test, feature = "test-support"))] mod` is not in the API a release
  compiles. `public_api::exports` walks from the crate root and follows
  re-exports; use it rather than `Workspace::sources` for anything asserting
  about shared vocabulary.
- **Check the rule against the repo before trusting it.** `generic-module-name`
  first flagged every `task/models.rs` beside `task/schemas.rs`, which is a
  deliberate convention: the directory names the domain and the leaves split
  entity from wire shape. A rule that fights a convention is wrong about the
  convention until proven otherwise.

## Dependencies

`anyhow`, `serde_json`, `syn`, `walkdir` — every one already in the lockfile,
so this crate adds no third-party build. The Cargo graph is read by shelling
out to `cargo metadata` rather than by adding `cargo_metadata`, for the same
reason. Keep it that way: a rule crate that slows the build gets deleted.

`guppy` is the right upgrade if graph rules outgrow `Workspace::reaches`. It
is a large dependency, so it waits until a rule needs it.
