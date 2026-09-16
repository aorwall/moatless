//! Which `pr view` calls this path claims, and what it renders for each — above
//! all that it never renders nothing.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    std::iter::once("moat-gh")
        .chain(args.iter().copied())
        .map(ToString::to_string)
        .collect()
}

#[test]
fn claims_a_comments_call_in_every_spelling() {
    for args in [
        argv(&["--comments"]),
        argv(&["897", "--comments"]),
        argv(&["897", "--comments", "-R", "o/r"]),
        argv(&["--comments", "--repo", "o/r", "feat/x"]),
    ] {
        assert!(
            parse(args.clone()).is_some(),
            "expected a claim for {args:?}"
        );
    }
}

/// Without `--comments` real `gh` already prints the title and body, and with a
/// flag we do not model it may print something this renderer cannot.
#[test]
fn leaves_every_other_pr_view_to_real_gh() {
    for args in [
        argv(&["897"]),
        argv(&[]),
        argv(&["897", "--json", "body,comments"]),
        argv(&["897", "--comments", "--json", "body"]),
        argv(&["897", "--comments", "--jq", ".body"]),
        argv(&["897", "--comments", "--web"]),
        argv(&["897", "--comments", "--template", "{{.body}}"]),
    ] {
        assert!(
            parse(args.clone()).is_none(),
            "expected no claim for {args:?}"
        );
    }
}

fn header() -> Header {
    Header {
        number: 42,
        title: "fix: x".into(),
        state: "OPEN".into(),
        is_draft: false,
        author: Some(GraphqlAuthor {
            login: "alice".into(),
        }),
        url: "https://github.com/o/r/pull/42".into(),
        head_ref_name: "feat/x".into(),
        base_ref_name: "main".into(),
        created_at: "2026-09-01T00:00:00Z".into(),
        body: "the description".into(),
    }
}

fn user(login: &str) -> RestUser {
    RestUser {
        login: login.into(),
    }
}

fn empty(header: Header) -> Conversation {
    Conversation {
        header,
        comments: Vec::new(),
        reviews: Vec::new(),
        review_comments: Vec::new(),
    }
}

/// The defect this path exists for: real `gh` answers this case with zero bytes
/// and exit 0, which a reviewer reads as a pull request that does not exist.
#[test]
fn a_pull_request_with_no_conversation_still_renders_the_pull_request() {
    let out = render(&empty(header()));
    assert!(out.contains("title:\tfix: x"), "{out}");
    assert!(out.contains("the description"), "{out}");
    assert!(
        out.contains("no comments, reviews, or review comments on this pull request"),
        "{out}"
    );
}

#[test]
fn an_empty_description_says_so_rather_than_rendering_blank() {
    let mut header = header();
    header.body = String::new();
    assert!(render(&empty(header)).contains("(no description)"));
}

#[test]
fn the_conversation_renders_in_the_order_it_happened() {
    let conversation = Conversation {
        header: header(),
        comments: vec![IssueComment {
            id: 3,
            user: Some(user("carol")),
            body: "third".into(),
            created_at: "2026-09-03T00:00:00Z".into(),
        }],
        reviews: vec![Review {
            id: 1,
            user: Some(user("alice")),
            body: "first".into(),
            state: "CHANGES_REQUESTED".into(),
            submitted_at: "2026-09-01T00:00:00Z".into(),
        }],
        review_comments: vec![ReviewComment {
            id: 2,
            user: Some(user("bob")),
            body: "second".into(),
            created_at: "2026-09-02T00:00:00Z".into(),
            path: "crates/x.rs".into(),
            line: Some(12),
            original_line: None,
            in_reply_to_id: None,
        }],
    };

    let out = render(&conversation);
    let at = |needle: &str| {
        out.find(needle)
            .unwrap_or_else(|| panic!("{needle} in {out}"))
    };
    assert!(
        at("first") < at("second") && at("second") < at("third"),
        "{out}"
    );
    assert!(
        out.contains("review by @alice — CHANGES_REQUESTED"),
        "{out}"
    );
    assert!(out.contains("crates/x.rs:12"), "{out}");
}

/// The two id spaces take different commands, and a delivered PR message labels
/// both `comment id`, so the heading has to say which one this is.
#[test]
fn each_comment_names_the_command_that_replies_to_it() {
    let conversation = Conversation {
        header: header(),
        comments: vec![IssueComment {
            id: 500,
            user: Some(user("carol")),
            body: "top level".into(),
            created_at: "2026-09-03T00:00:00Z".into(),
        }],
        reviews: Vec::new(),
        review_comments: vec![ReviewComment {
            id: 600,
            user: Some(user("bob")),
            body: "inline".into(),
            created_at: "2026-09-02T00:00:00Z".into(),
            path: "crates/x.rs".into(),
            line: None,
            original_line: Some(7),
            in_reply_to_id: Some(599),
        }],
    };

    let out = render(&conversation);
    assert!(
        out.contains("conversation comment by @carol at 2026-09-03T00:00:00Z (comment id: 500) — reply with `moat gh pr comment 42`"),
        "{out}"
    );
    assert!(
        out.contains("review comment by @bob on crates/x.rs:7 at 2026-09-02T00:00:00Z (comment id: 600) — reply with `moat gh pr reply 42 600`, in reply to 599"),
        "{out}"
    );
}

/// GitHub wraps each batch of inline comments in a review with no prose of its
/// own; rendering those would put an empty entry above every inline comment.
#[test]
fn a_review_carrying_only_inline_comments_is_not_rendered_twice() {
    let conversation = Conversation {
        header: header(),
        comments: Vec::new(),
        reviews: vec![Review {
            id: 1,
            user: Some(user("bob")),
            body: "  ".into(),
            state: "COMMENTED".into(),
            submitted_at: "2026-09-02T00:00:00Z".into(),
        }],
        review_comments: vec![ReviewComment {
            id: 2,
            user: Some(user("bob")),
            body: "inline".into(),
            created_at: "2026-09-02T00:00:00Z".into(),
            path: "crates/x.rs".into(),
            line: Some(1),
            original_line: None,
            in_reply_to_id: None,
        }],
    };

    let out = render(&conversation);
    assert!(!out.contains("review by @bob"), "{out}");
    assert!(out.contains("review comment by @bob"), "{out}");
}

#[test]
fn a_slurped_page_list_flattens_and_an_empty_list_is_not_an_error() {
    let comments: Vec<IssueComment> = parse_pages(
        br#"[[{"id":1,"body":"a","created_at":"t"}],[{"id":2,"body":"b","created_at":"t"}]]"#,
    )
    .expect("two pages");
    assert_eq!(comments.len(), 2);

    let none: Vec<IssueComment> = parse_pages(b"[[]]").expect("one empty page");
    assert!(none.is_empty());
}

/// A missing author must not sink the whole read: the rest of the conversation
/// is what the reader came for.
#[test]
fn a_comment_from_a_deleted_account_still_renders() {
    let conversation = Conversation {
        header: header(),
        comments: vec![IssueComment {
            id: 1,
            user: None,
            body: "orphaned".into(),
            created_at: String::new(),
        }],
        reviews: Vec::new(),
        review_comments: Vec::new(),
    };
    let out = render(&conversation);
    assert!(out.contains("@unknown"), "{out}");
    assert!(out.contains("an unknown time"), "{out}");
    assert!(out.contains("orphaned"), "{out}");
}
