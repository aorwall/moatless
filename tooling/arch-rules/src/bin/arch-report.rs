// A report binary's whole output is stdout, the same reason `moatless-cli`
// allows these at its own root.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Print every architecture violation in the workspace.
//!
//! ```text
//! cargo run -p moatless-arch-rules --bin arch-report
//! cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline
//! cargo run -p moatless-arch-rules --bin arch-report -- --persistence-inventory
//! ```
//!
//! Exits 0 whatever it finds. Enforcement is the test harness's job, so this
//! stays usable as a to-do list while the counts come down.

#[path = "arch_report/report.rs"]
mod report;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let set = |flag: &str| args.iter().any(|a| a == flag);
    report::run(
        set("--update-baseline"),
        set("--json"),
        set("--persistence-inventory"),
    )
}
