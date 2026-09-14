// A report binary's whole output is stdout, the same reason `moatless-cli`
// allows these at its own root.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Prints every architecture violation in the workspace; exits 0 regardless.
//!
//! `--comments` reports the comment-block ceiling, which is not a registered
//! rule and is held by nothing: CI turns its `--findings` file into a review
//! comment. `tooling/arch-rules/AGENTS.md` spells out every flag.

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
        return comments::run(
            value("--since"),
            value("--findings").map(std::path::Path::new),
        );
    }
    // Both flags only mean anything above, and a typo there writes no file
    // rather than failing, which reaches CI as a bot that stopped commenting.
    if set("--findings") || set("--since") {
        anyhow::bail!("--findings and --since only apply to --comments");
    }

    report::run(
        set("--update-baseline"),
        set("--json"),
        set("--persistence-inventory"),
    )
}
