//! What `moat gh pr create` says will reach the task about the pull request it
//! bound, so the agent waits for those messages rather than polling GitHub.

/// The lines to print after a bind, from the response's `notified_events`.
/// `bound` is false when the task asked for a binding and got none.
pub(super) fn notice(bound: bool, events: &[String]) -> Vec<String> {
    if !bound {
        return vec![
            "Not bound: this task gets no messages about this PR, so check its CI and \
             reviews yourself."
                .to_string(),
        ];
    }
    if events.is_empty() {
        return Vec::new();
    }
    let labels: Vec<&str> = events.iter().map(|event| label(event)).collect();
    let mut lines = vec![format!(
        "This task is told of {}. Wait for those messages rather than watching runs \
         with `gh run watch`.",
        join(&labels)
    )];
    if events.iter().any(|event| event == "post_merge") {
        lines.push(
            "After the merge, one message says every workflow on the merge commit passed, \
             or one per run that failed. A cancelled run sends none."
                .to_string(),
        );
    } else {
        lines.push(
            "No post-merge pipeline message reaches this task: check the default branch's \
             runs yourself if the change needs them."
                .to_string(),
        );
    }
    lines
}

/// Unknown names print as themselves: the backend may add an event before the
/// CLI learns its label.
fn label(event: &str) -> &str {
    match event {
        "issue_comment" => "PR comments",
        "pull_request_review_comment" => "review comments",
        "pull_request_review" => "reviews",
        "check_run" => "failing checks",
        "merge_conflict" => "merge conflicts",
        "pr_merged" => "the merge",
        "post_merge" => "the post-merge pipeline result",
        other => other,
    }
}

fn join(labels: &[&str]) -> String {
    match labels {
        [] => String::new(),
        [only] => (*only).to_string(),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::notice;

    fn events(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_task_told_of_the_post_merge_result_is_told_not_to_watch_runs() {
        let lines = notice(true, &events(&["check_run", "pr_merged", "post_merge"]));
        assert_eq!(
            lines[0],
            "This task is told of failing checks, the merge, and the post-merge pipeline \
             result. Wait for those messages rather than watching runs with `gh run watch`."
        );
        assert!(lines[1].contains("every workflow on the merge commit passed"));
        assert!(lines[1].contains("A cancelled run sends none"));
    }

    #[test]
    fn a_task_closing_with_the_pr_is_told_no_post_merge_message_comes() {
        let lines = notice(true, &events(&["issue_comment", "check_run"]));
        assert!(lines[1].starts_with("No post-merge pipeline message reaches this task"));
    }

    #[test]
    fn an_unbound_task_is_told_it_hears_nothing() {
        assert!(notice(false, &[])[0].starts_with("Not bound"));
    }

    #[test]
    fn an_older_backend_that_lists_no_events_adds_nothing() {
        assert!(notice(true, &[]).is_empty());
    }
}
