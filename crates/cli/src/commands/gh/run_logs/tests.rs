//! Which `run view` calls this path claims, which `gh api` reads get the escape
//! flag, and which jobs `--log-failed` selects.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    std::iter::once("moat-gh")
        .chain(args.iter().copied())
        .map(ToString::to_string)
        .collect()
}

#[test]
fn claims_every_log_reading_run_view() {
    for args in [
        argv(&["123", "--log"]),
        argv(&["123", "--log-failed"]),
        argv(&["--log-failed", "-R", "o/r"]),
        argv(&["--job", "456", "--log"]),
    ] {
        assert!(
            parse(args.clone()).is_some(),
            "expected a claim for {args:?}"
        );
    }
}

/// A status read is what real `gh` renders best, and a flag we do not model may
/// change what the call is asking for.
#[test]
fn leaves_every_other_run_view_to_real_gh() {
    for args in [
        argv(&["123"]),
        argv(&["123", "--json", "jobs"]),
        argv(&["123", "--log", "--exit-status"]),
        argv(&["123", "--web"]),
    ] {
        assert!(
            parse(args.clone()).is_none(),
            "expected no claim for {args:?}"
        );
    }
}

#[test]
fn an_actions_log_read_gets_the_escape_flag_once() {
    let args = argv(&["api", "repos/o/r/actions/jobs/9/logs"]);
    let with_flag = allow_escape_sequences(&args).expect("flag added");
    assert_eq!(
        with_flag.last().map(String::as_str),
        Some("--allow-escape-sequences")
    );
    assert!(
        allow_escape_sequences(&with_flag).is_none(),
        "a call already carrying the flag is left alone"
    );
}

#[test]
fn only_an_actions_log_endpoint_gets_the_escape_flag() {
    for endpoint in [
        "repos/o/r/actions/runs/9/logs",
        "/repos/o/r/actions/jobs/9/logs",
        "https://api.github.com/repos/o/r/actions/jobs/9/logs?per_page=1",
    ] {
        assert!(reads_actions_log(endpoint), "{endpoint}");
    }
    for endpoint in [
        "repos/o/r/issues/9/comments",
        "repos/o/r/actions/runs/9/jobs",
        "repos/o/r/logs",
    ] {
        assert!(!reads_actions_log(endpoint), "{endpoint}");
    }
}

fn job(name: &str, status: &str, conclusion: Option<&str>) -> Job {
    Job {
        id: 1,
        name: name.to_string(),
        status: status.to_string(),
        conclusion: conclusion.map(ToString::to_string),
    }
}

/// A cancelled job stopped because something else failed, and a queued one has
/// no log yet; neither is what `--log-failed` was asked for.
#[test]
fn only_a_failed_or_timed_out_job_is_selected_by_log_failed() {
    assert!(job("a", "completed", Some("failure")).failed());
    assert!(job("a", "completed", Some("timed_out")).failed());
    assert!(!job("a", "completed", Some("success")).failed());
    assert!(!job("a", "completed", Some("cancelled")).failed());
    assert!(!job("a", "in_progress", None).failed());
}

/// The status stands in for a job that has not concluded, so a heading never
/// reads `job id 1, ` with nothing after it.
#[test]
fn a_job_that_has_not_concluded_reports_its_status() {
    assert_eq!(job("a", "in_progress", None).outcome(), "in_progress");
    assert!(!job("a", "in_progress", None).complete());
    assert_eq!(job("a", "completed", Some("failure")).outcome(), "failure");
    assert!(job("a", "completed", Some("failure")).complete());
}

#[test]
fn every_job_in_a_slurped_page_list_is_read() {
    let pages: Vec<JobPage> = serde_json::from_slice(
        br#"[{"total_count":2,"jobs":[{"id":1,"name":"a","status":"completed","conclusion":"failure"}]},
             {"total_count":2,"jobs":[{"id":2,"name":"b","status":"queued","conclusion":null}]}]"#,
    )
    .expect("two pages");
    let jobs: Vec<Job> = pages.into_iter().flat_map(|page| page.jobs).collect();
    assert_eq!(jobs.len(), 2);
    assert!(jobs[0].failed());
    assert_eq!(jobs[1].outcome(), "queued");
}
