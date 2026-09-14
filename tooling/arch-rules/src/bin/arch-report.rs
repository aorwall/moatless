// A report binary's whole output is stdout, the same reason `moatless-cli`
// allows these at its own root.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Prints every architecture violation in the workspace; exits 0 regardless.
//!
//! `--comments` is the exception: it reports the comment-block ceiling, which
//! is not a registered rule, and exits 1 on a hit so a CI step can gate on it.
//! `tooling/arch-rules/AGENTS.md` spells out every flag.

#[path = "arch_report/comments.rs"]
mod comments;
#[path = "arch_report/report.rs"]
mod report;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let set = |flag: &str| args.iter().any(|a| a == flag);
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };

    if set("--comments") {
        if comments::run(value("--since"))? {
            std::process::exit(1);
        }
        return Ok(());
    }

    report::run(
        set("--update-baseline"),
        set("--json"),
        set("--persistence-inventory"),
    )
}
