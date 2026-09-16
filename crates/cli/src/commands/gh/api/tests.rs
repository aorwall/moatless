//! Which `gh api` call created something worth recording, and which id it gave.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

/// The shape a `gh api …` argument list creates, if any. `line` is written the
/// way an agent types it, starting at `api`.
fn api_kind(line: &str) -> Option<ApiCreated> {
    let args: Vec<String> = line.split_whitespace().map(ToString::to_string).collect();
    assert_eq!(
        args.first().map(String::as_str),
        Some("api"),
        "not a gh api call"
    );
    parse_api_creation(&args[1..]).map(|c| c.kind)
}

#[test]
fn api_creating_an_inline_review_comment_is_recognised() {
    // The shape the reviewer loop's prompt tells agents to use.
    let args = argv(&[
        "--method",
        "POST",
        "repos/soaplabs/soap-config/pulls/42/comments",
        "-f",
        "body=needs a guard",
        "-f",
        "commit_id=abc123",
        "-F",
        "line=12",
    ]);
    assert_eq!(
        parse_api_creation(&args),
        Some(ApiCreation {
            owner: "soaplabs".into(),
            repo: "soap-config".into(),
            number: 42,
            kind: ApiCreated::ReviewComment,
        })
    );
    // Rooted path, a header, and the endpoint after the flags.
    assert_eq!(
        api_kind(
            "api -H Accept:application/vnd.github+json -X POST \
             /repos/o/r/pulls/7/comments --input /tmp/c.json"
        ),
        Some(ApiCreated::ReviewComment)
    );
    // Implicit POST from `-f`, endpoint last.
    assert_eq!(
        api_kind("api -f body=hi repos/o/r/pulls/7/comments"),
        Some(ApiCreated::ReviewComment)
    );
}

#[test]
fn a_submitted_review_names_the_endpoint_listing_its_own_comments() {
    let created = parse_api_creation(&argv(&[
        "--method",
        "POST",
        "repos/soaplabs/soap-config/pulls/42/reviews",
        "-f",
        "event=COMMENT",
    ]))
    .expect("a review submission");
    assert_eq!(
        review_comments_endpoint(&created, 555),
        "repos/soaplabs/soap-config/pulls/42/reviews/555/comments"
    );
}

#[test]
fn every_comment_in_a_slurped_page_list_is_read_and_a_broken_body_reads_none() {
    assert_eq!(
        ids_in_comment_list(br#"[[{"id": 1}, {"id": 2}], [{"id": 3}]]"#),
        vec![1, 2, 3]
    );
    assert_eq!(ids_in_comment_list(b"[[]]"), Vec::<u64>::new());
    // What `--paginate` prints without `--slurp`, and an error message.
    assert_eq!(
        ids_in_comment_list(br#"[{"id": 1}][{"id": 2}]"#),
        Vec::<u64>::new()
    );
    assert_eq!(
        ids_in_comment_list(b"gh: Not Found (HTTP 404)"),
        Vec::<u64>::new()
    );
}

#[test]
fn api_submitting_a_review_is_recognised() {
    assert_eq!(
        api_kind("api repos/o/r/pulls/42/reviews -f event=COMMENT -f body=LGTM"),
        Some(ApiCreated::Review)
    );
    assert_eq!(
        api_kind(
            "api --method=POST https://api.github.com/repos/o/r/pulls/42/reviews \
             --input /tmp/r.json"
        ),
        Some(ApiCreated::Review)
    );
    // The approve step of the code-review loop.
    assert_eq!(
        api_kind("api repos/OWNER/REPO/pulls/42/reviews --method POST --field event=APPROVE"),
        Some(ApiCreated::Review)
    );
}

#[test]
fn api_creating_an_issue_comment_is_recognised() {
    assert_eq!(
        api_kind("api -X POST repos/o/r/issues/42/comments -f body=hi"),
        Some(ApiCreated::IssueComment)
    );
}

#[test]
fn api_reads_record_nothing() {
    // No body flags — `gh api` defaults to GET.
    assert!(api_kind("api repos/o/r/pulls/42/comments").is_none());
    assert!(api_kind("api repos/o/r/issues/42/comments --paginate").is_none());
    // Explicit read, even with a body flag present.
    assert!(api_kind("api --method GET repos/o/r/pulls/42/reviews -f per_page=100").is_none());
    assert!(api_kind("api -X GET repos/o/r/issues/42/comments").is_none());
}

#[test]
fn api_writes_to_unrelated_resources_claim_nothing() {
    assert!(api_kind("api -X POST repos/o/r/issues/42/labels -f labels[]=bug").is_none());
    assert!(api_kind("api --method POST repos/o/r/pulls/42/merge").is_none());
    assert!(api_kind("api -X POST repos/o/r/issues -f title=bug").is_none());
    assert!(api_kind("api -X POST graphql -f query={}").is_none());
    // A non-numeric PR number is not an endpoint we recognise.
    assert!(api_kind("api -X POST repos/o/r/pulls/comments -f body=x").is_none());
    // Threaded replies go through the typed proxy; the deeper path must not
    // be mistaken for a new inline comment.
    assert!(api_kind("api -X POST repos/o/r/pulls/42/comments/123/replies -f body=x").is_none());
    // An edit reuses an existing id and produces no new event.
    assert!(api_kind("api -X PATCH repos/o/r/issues/42/comments -f body=x").is_none());
}

#[test]
fn created_id_read_from_json_response() {
    let body = r#"{
      "url": "https://api.github.com/repos/o/r/pulls/comments/987",
      "id": 987,
      "pull_request_review_id": 555,
      "html_url": "https://github.com/o/r/pull/42#discussion_r987"
    }"#;
    assert_eq!(parse_created_id(body, ApiCreated::ReviewComment), Some(987));
}

#[test]
fn created_id_falls_back_to_the_html_url_anchor() {
    // Output the JSON parser cannot take — a preceding line from the same
    // command, or a truncated body.
    let review_comment = "posting…\n{\"id\": 987, \"html_url\": \
                          \"https://github.com/o/r/pull/42#discussion_r987\"";
    assert_eq!(
        parse_created_id(review_comment, ApiCreated::ReviewComment),
        Some(987)
    );

    let review = "…\"html_url\":\"https://github.com/o/r/pull/42#pullrequestreview-555\",";
    assert_eq!(parse_created_id(review, ApiCreated::Review), Some(555));

    let issue_comment = "…\"html_url\":\"https://github.com/o/r/issues/42#issuecomment-111\",";
    assert_eq!(
        parse_created_id(issue_comment, ApiCreated::IssueComment),
        Some(111)
    );
}

#[test]
fn created_id_absent_from_unusable_output() {
    // A list response is not an object, and carries no anchor for the shape
    // we asked about.
    assert!(parse_created_id("[{\"id\": 1}]", ApiCreated::Review).is_none());
    assert!(parse_created_id("", ApiCreated::IssueComment).is_none());
    assert!(parse_created_id("gh: Not Found (HTTP 404)", ApiCreated::ReviewComment).is_none());
    // The anchor must match the shape — a review submission's response does
    // not name a review comment.
    assert!(
        parse_created_id(
            "\"html_url\":\"https://github.com/o/r/pull/42#pullrequestreview-555\"",
            ApiCreated::ReviewComment
        )
        .is_none()
    );
}

#[test]
fn the_claimed_id_comes_from_the_response_not_the_request() {
    // The endpoint names PR 42; the claim names comment 987, which only the
    // response can supply.
    let created = parse_api_creation(&argv(&[
        "-X",
        "POST",
        "repos/o/r/pulls/42/comments",
        "-f",
        "body=x",
        "-F",
        "line=1",
    ]))
    .unwrap();
    let id = parse_created_id(
        r#"{"id": 987, "pull_request_review_id": 555}"#,
        created.kind,
    )
    .unwrap();
    assert!(matches!(
        created.kind.event_subject(id),
        GithubExternalEventSubject::ReviewComment(987)
    ));
}

// =======================================================================
// What a creating `gh api` call may not do to its own response
// =======================================================================

fn filtered_by(line: &str) -> Option<String> {
    output_replacing_flag(&argv(&line.split_whitespace().collect::<Vec<_>>()))
}

/// The reported silent gap: `--jq .id` is the natural flag for an agent
/// posting an inline comment, and it leaves nothing for the id to be read
/// from.
#[test]
fn a_filter_over_the_response_is_recognised_in_every_spelling() {
    let posting = "-X POST repos/o/r/pulls/42/comments";
    assert_eq!(
        filtered_by(&format!("{posting} --jq .id")).as_deref(),
        Some("--jq")
    );
    assert_eq!(
        filtered_by("--jq=.id repos/o/r/issues/42/comments").as_deref(),
        Some("--jq=.id")
    );
    assert_eq!(
        filtered_by("-q .id repos/o/r/issues/42/comments").as_deref(),
        Some("-q")
    );
    assert_eq!(
        filtered_by("-q.id repos/o/r/issues/42/comments").as_deref(),
        Some("-q.id")
    );
    assert_eq!(
        filtered_by("--template {{.id}} x").as_deref(),
        Some("--template")
    );
    assert_eq!(filtered_by("-t {{.id}} x").as_deref(), Some("-t"));
    assert_eq!(filtered_by("--silent x").as_deref(), Some("--silent"));
}

#[test]
fn an_unfiltered_call_is_left_alone() {
    assert_eq!(
        filtered_by("-X POST repos/o/r/pulls/42/comments -f body=x"),
        None
    );
    // A field value spelled like a filter is a value, not a flag.
    assert_eq!(
        output_replacing_flag(&argv(&[
            "-X",
            "POST",
            "repos/o/r/issues/42/comments",
            "-f",
            "body=--jq",
            "-f",
            "line=-q",
        ])),
        None
    );
}

#[test]
fn the_refusal_names_the_flag_and_that_nothing_was_posted() {
    let message = refuse_filtered_write("--jq", ApiCreated::ReviewComment);
    assert!(message.contains("`--jq`"), "{message}");
    assert!(message.contains("review comment"), "{message}");
    assert!(message.contains("Nothing was posted"), "{message}");
    assert!(message.contains("--no-record"), "{message}");
}

#[test]
fn no_record_is_taken_out_before_gh_sees_it() {
    let (args, skipped) = strip_no_record(&argv(&[
        "api",
        "repos/o/r/issues/42/comments",
        "--method",
        "POST",
        "--no-record",
        "-f",
        "body=x",
    ]));
    assert!(skipped);
    assert_eq!(
        args,
        argv(&[
            "api",
            "repos/o/r/issues/42/comments",
            "--method",
            "POST",
            "-f",
            "body=x"
        ])
    );
}

#[test]
fn a_body_that_reads_no_record_is_still_a_body() {
    let original = argv(&[
        "api",
        "repos/o/r/issues/42/comments",
        "-X",
        "POST",
        "-f",
        "body=--no-record",
    ]);
    let (args, skipped) = strip_no_record(&original);
    assert!(!skipped);
    assert_eq!(args, original);
}
