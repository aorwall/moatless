//! What `moat gh` decides before it shells out: which subcommand is typed,
//! which `gh api` call created something worth recording, and where the
//! repository and the token come from.

use super::*;

#[test]
fn github_child_status_remains_available_to_the_executable() {
    assert!(super::command_status(0).is_ok());
    for code in [1, 2, 42, 127] {
        let error = super::command_status(code).expect_err("nonzero child status");
        let status = error
            .downcast_ref::<super::CommandExit>()
            .expect("typed child status");
        assert_eq!(status.0, code);
    }
}

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[test]
fn parses_pr_create_with_typed_flags() {
    let parsed = parse_typed(&argv(&[
        "pr", "create", "--title", "fix: x", "--body", "b", "--head", "feat/x", "--base", "main",
        "--draft", "-R", "o/r", "--task", "t_1",
    ]));
    let Some(TypedInvocation::PrCreate(a)) = parsed else {
        panic!("expected typed pr create, got {parsed:?}");
    };
    assert_eq!(a.title, "fix: x");
    assert_eq!(a.body.as_deref(), Some("b"));
    assert_eq!(a.head.as_deref(), Some("feat/x"));
    assert_eq!(a.base.as_deref(), Some("main"));
    assert!(a.draft);
    assert_eq!(a.repo.as_deref(), Some("o/r"));
    assert_eq!(a.task.as_deref(), Some("t_1"));
}

#[test]
fn parses_pr_create_short_flags() {
    let parsed = parse_typed(&argv(&["pr", "create", "-t", "title", "-b", "body"]));
    assert!(matches!(parsed, Some(TypedInvocation::PrCreate(_))));
}

#[test]
fn pr_create_with_unknown_flag_falls_through() {
    // `--fill` is not typed — must go to the real gh.
    assert_eq!(parse_typed(&argv(&["pr", "create", "--fill"])), None);
    // Missing required --title also falls through.
    assert_eq!(parse_typed(&argv(&["pr", "create"])), None);
}

/// `--body-file` must not demote `pr create` to plain `gh`: plain `gh` opens
/// the PR and returns its URL while binding nothing, so the PR is never
/// connected to the task even though comments posted in the same run are.
#[test]
fn pr_create_with_body_file_stays_on_the_typed_path() {
    let parsed = parse_typed(&argv(&[
        "pr",
        "create",
        "-t",
        "t",
        "--body-file",
        "/tmp/b.md",
        "--head",
        "feat/x",
        "--base",
        "main",
    ]));
    let Some(TypedInvocation::PrCreate(a)) = parsed else {
        panic!("expected typed pr create, got {parsed:?}");
    };
    assert_eq!(
        a.body_file.as_deref(),
        Some(std::path::Path::new("/tmp/b.md"))
    );
    assert!(a.body.is_none());
}

/// `-F` is what real `gh` spells `--body-file`, so an agent copying a gh
/// invocation lands on the typed path too.
#[test]
fn pr_create_accepts_the_gh_short_body_file_flag() {
    let parsed = parse_typed(&argv(&["pr", "create", "-t", "t", "-F", "/tmp/b.md"]));
    assert!(matches!(parsed, Some(TypedInvocation::PrCreate(_))));
}

/// Both spellings at once is a user error, not a demotion — but it still falls
/// through, so real gh reports it rather than moat guessing.
#[test]
fn pr_create_rejects_body_and_body_file_together() {
    assert_eq!(
        parse_typed(&argv(&[
            "pr",
            "create",
            "-t",
            "t",
            "-b",
            "inline",
            "--body-file",
            "/tmp/b.md"
        ])),
        None
    );
}

/// A comment body may come from a file as well; the same demotion cost applied,
/// and the same flag caused it.
#[test]
fn comment_accepts_body_file() {
    let parsed = parse_typed(&argv(&["pr", "comment", "42", "--body-file", "/tmp/b.md"]));
    let Some(TypedInvocation::Comment(a)) = parsed else {
        panic!("expected typed comment, got {parsed:?}");
    };
    assert_eq!(a.number, 42);
    assert!(a.body.is_none());
    assert_eq!(
        a.body_file.as_deref(),
        Some(std::path::Path::new("/tmp/b.md"))
    );
}

/// A comment still needs a body from somewhere.
#[test]
fn comment_without_any_body_falls_through() {
    assert_eq!(parse_typed(&argv(&["pr", "comment", "42"])), None);
}

#[test]
fn body_file_is_read_when_no_inline_body_is_given() {
    let dir = std::env::temp_dir().join("moat-gh-body-file-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("body.md");
    std::fs::write(&path, "from a file").unwrap();

    assert_eq!(
        read_body(None, Some(path.as_path())).unwrap().as_deref(),
        Some("from a file")
    );
    // An inline body wins; clap has already ruled out both being present.
    assert_eq!(
        read_body(Some("inline".into()), Some(path.as_path()))
            .unwrap()
            .as_deref(),
        Some("inline")
    );
    std::fs::remove_file(&path).ok();
}

/// A missing file names the path rather than failing as a bare io error — the
/// agent needs to know which file it meant.
#[test]
fn a_missing_body_file_names_the_path() {
    let err = read_body(None, Some(std::path::Path::new("/nonexistent/body.md")))
        .expect_err("a missing body file is an error");
    assert!(
        err.to_string().contains("/nonexistent/body.md"),
        "error should name the file, got: {err}"
    );
}

/// The demotion warning exists so a fall-through is never silent again.
/// `--help` is not a demotion: the user asked for text.
#[test]
fn help_is_not_reported_as_a_demotion() {
    assert_eq!(parse_typed(&argv(&["pr", "create", "--help"])), None);
}

#[test]
fn parses_pr_and_issue_comment() {
    let parsed = parse_typed(&argv(&["pr", "comment", "42", "--body", "hi"]));
    let Some(TypedInvocation::Comment(a)) = parsed else {
        panic!("expected typed comment, got {parsed:?}");
    };
    assert_eq!(a.number, 42);
    assert_eq!(a.body.as_deref(), Some("hi"));

    assert!(matches!(
        parse_typed(&argv(&["issue", "comment", "7", "-b", "hi", "-R", "o/r"])),
        Some(TypedInvocation::Comment(_))
    ));
}

#[test]
fn comment_with_non_numeric_target_falls_through() {
    // gh accepts URLs/branches as the comment target — we only type numbers.
    assert_eq!(
        parse_typed(&argv(&[
            "pr",
            "comment",
            "https://github.com/o/r/pull/42",
            "--body",
            "hi"
        ])),
        None
    );
    // No --body → editor-based flow → passthrough.
    assert_eq!(parse_typed(&argv(&["pr", "comment", "42"])), None);
}

#[test]
fn parses_pr_reply() {
    let parsed = parse_typed(&argv(&[
        "pr", "reply", "42", "777", "--body", "fixed", "-R", "o/r", "--task", "t_1",
    ]));
    let Some(TypedInvocation::PrReply(a)) = parsed else {
        panic!("expected typed pr reply, got {parsed:?}");
    };
    assert_eq!(a.pr_number, 42);
    assert_eq!(a.comment_id, 777);
    assert_eq!(a.body.as_deref(), Some("fixed"));
    assert_eq!(a.repo.as_deref(), Some("o/r"));
    assert_eq!(a.task.as_deref(), Some("t_1"));
}

#[test]
fn pr_reply_missing_body_falls_through() {
    // No --body → fall through to the real gh (which errors: `pr reply`
    // is moat-only). Better than a half-typed editor flow.
    assert_eq!(parse_typed(&argv(&["pr", "reply", "42", "777"])), None);
}

#[test]
fn pr_reply_non_numeric_positionals_fall_through() {
    assert_eq!(
        parse_typed(&argv(&["pr", "reply", "abc", "777", "--body", "x"])),
        None
    );
    assert_eq!(
        parse_typed(&argv(&["pr", "reply", "42", "not-an-id", "--body", "x"])),
        None
    );
    // Missing comment_id entirely also falls through.
    assert_eq!(
        parse_typed(&argv(&["pr", "reply", "42", "--body", "x"])),
        None
    );
}

#[test]
fn parses_stack_invocation_and_strips_task_flag() {
    let parsed = parse_typed(&argv(&[
        "stack", "link", "207", "208", "--task", "t_1", "--open",
    ]));
    let Some(TypedInvocation::Stack(a)) = parsed else {
        panic!("expected stack invocation, got {parsed:?}");
    };
    assert_eq!(a.task.as_deref(), Some("t_1"));
    assert_eq!(a.args, argv(&["stack", "link", "207", "208", "--open"]));

    let parsed = parse_typed(&argv(&["stack", "submit", "--task=t_1", "--auto"]));
    let Some(TypedInvocation::Stack(a)) = parsed else {
        panic!("expected stack invocation, got {parsed:?}");
    };
    assert_eq!(a.task.as_deref(), Some("t_1"));
    assert_eq!(a.args, argv(&["stack", "submit", "--auto"]));
}

#[test]
fn stack_missing_task_value_falls_through() {
    assert_eq!(parse_typed(&argv(&["stack", "link", "1", "--task"])), None);
    assert_eq!(parse_typed(&argv(&["stack", "link", "1", "--task="])), None);
}

#[test]
fn empty_task_id_is_absent_for_stack_binding() {
    assert_eq!(non_empty_task_id(None), None);
    assert_eq!(non_empty_task_id(Some(String::new())), None);
    assert_eq!(non_empty_task_id(Some("  ".into())), None);
    assert_eq!(non_empty_task_id(Some("t_1".into())), Some("t_1".into()));
}

#[test]
fn extracts_stack_link_operands_around_flags() {
    assert_eq!(
        stack_link_operands(&argv(&[
            "--base",
            "main",
            "207",
            "--remote=origin",
            "feat/top",
            "--open"
        ])),
        argv(&["207", "feat/top"])
    );
}

#[test]
fn parses_pull_request_urls() {
    assert_eq!(
        parse_pull_request_url("https://github.com/soaplabs/moatless/pull/208"),
        Some(PullRequestRef {
            owner: "soaplabs".into(),
            repo: "moatless".into(),
            number: 208,
        })
    );
    assert_eq!(
        parse_pull_request_url("https://github.com/soaplabs/moatless/issues/208"),
        None
    );
    assert_eq!(parse_pull_request_url("not-a-url"), None);
}

#[test]
fn other_subcommands_fall_through() {
    assert_eq!(parse_typed(&argv(&["pr", "view", "42"])), None);
    assert_eq!(parse_typed(&argv(&["api", "/user"])), None);
    assert_eq!(parse_typed(&argv(&["run", "watch", "1"])), None);
    assert_eq!(parse_typed(&argv(&["issue", "create", "-t", "x"])), None);
    assert_eq!(parse_typed(&argv(&[])), None);
    assert_eq!(parse_typed(&argv(&["pr"])), None);
}

#[test]
fn parse_repo_arg_accepts_slug_and_urls() {
    assert_eq!(parse_repo_arg("o/r"), Some(("o".into(), "r".into())));
    assert_eq!(
        parse_repo_arg("https://github.com/o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(
        parse_repo_arg("git@github.com:o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(parse_repo_arg("just-a-name"), None);
    assert_eq!(parse_repo_arg("a/b/c"), None);
    assert_eq!(parse_repo_arg("/r"), None);
}

#[test]
fn parse_github_remote_forms() {
    assert_eq!(
        parse_github_remote("https://github.com/soaplabs/soap-backend.git"),
        Some(("soaplabs".into(), "soap-backend".into()))
    );
    assert_eq!(
        parse_github_remote("ssh://git@github.com/o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(parse_github_remote("https://gitlab.com/o/r.git"), None);
}

#[test]
fn a_proxied_origin_is_recognized_whichever_url_names_the_backend() {
    assert_eq!(
        any_host_proxy_repository_id(
            "http://moatless-vibe-backend.moatless.svc.cluster.local:8080/git/repo_1"
        ),
        Some("repo_1")
    );
    assert_eq!(
        any_host_proxy_repository_id("https://moatless.example.com/git/repo_1.git"),
        Some("repo_1")
    );
    assert_eq!(
        any_host_proxy_repository_id("https://github.com/soaplabs/moatless.git"),
        None,
        "a real git host names an owner, not a repository id"
    );
    assert_eq!(any_host_proxy_repository_id("https://host/git/"), None);
}

/// The backend a proxied origin is cloned from.
const BACKEND: &str = "https://moatless.example.com";

/// `gh` left to itself prefers a remote named `upstream`, so in a fork clone
/// that has both, this is the difference between reading the fork's PR and
/// reading whatever upstream PR shares its number.
#[test]
fn a_fork_s_origin_names_the_fork_whatever_else_it_has_a_remote_for() {
    assert!(matches!(
        classify_origin(BACKEND, "https://github.com/soaplabs/t3code.git"),
        OriginUrl::Github(owner, name) if owner == "soaplabs" && name == "t3code"
    ));
}

#[test]
fn a_proxied_origin_names_the_id_to_read_back() {
    assert!(matches!(
        classify_origin(BACKEND, "https://moatless.example.com/git/repo_1"),
        OriginUrl::Proxied("repo_1")
    ));
}

#[test]
fn an_origin_on_another_host_names_no_repository() {
    assert!(matches!(
        classify_origin(BACKEND, "https://forgejo.example.com/o/r.git"),
        OriginUrl::Foreign
    ));
}

// =======================================================================
// `gh api` recognition
// =======================================================================

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

// =======================================================================
// Created-id read-back
// =======================================================================

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

/// The reported 401: `GH_TOKEN` is seeded at provision and expires with the
/// installation token behind it, so in a sandbox the broker's per-call token
/// has to outrank it.
#[test]
fn the_broker_outranks_a_stale_env_token() {
    assert_eq!(
        resolve_gh_token(true, true, || Some("fresh".to_string())),
        Some("fresh".to_string())
    );
}

#[test]
fn an_exported_env_token_still_wins_off_the_broker() {
    assert_eq!(
        resolve_gh_token(false, true, || Some("helper".to_string())),
        None
    );
}

fn gh_repo_env(gh_repo: Option<&str>) -> Option<String> {
    gh_command(&argv(&["pr", "view"]), None, gh_repo)
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new("GH_REPO"))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
}

/// Without this, `gh` resolves the repository itself: it prefers an `upstream`
/// remote over `origin`, and a proxied origin names no owner at all.
#[test]
fn the_resolved_repository_reaches_gh_as_gh_repo() {
    assert_eq!(
        gh_repo_env(Some("owner/repo")).as_deref(),
        Some("owner/repo")
    );
}

#[test]
fn an_origin_that_resolved_to_nothing_leaves_the_environment_alone() {
    assert_eq!(gh_repo_env(None), None);
}

#[test]
fn the_credential_helper_answers_when_no_env_token_is_set() {
    assert_eq!(
        resolve_gh_token(false, false, || Some("helper".to_string())),
        Some("helper".to_string())
    );
}

/// A broker read that fails leaves `gh` to find the ambient `GH_TOKEN`, which
/// beats sending the call unauthenticated.
#[test]
fn a_failed_broker_read_falls_back_to_gh_s_own_resolution() {
    assert_eq!(resolve_gh_token(true, true, || None), None);
}
