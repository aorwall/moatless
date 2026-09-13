// A report binary's whole output is stdout, the same reason `moatless-cli`
// allows these at its own root.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Prints every architecture violation in the workspace; exits 0 regardless.
//!
//! ```text
//! cargo run -p moatless-arch-rules --bin arch-report
//! cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline
//! cargo run -p moatless-arch-rules --bin arch-report -- --persistence-inventory
//! ```

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
