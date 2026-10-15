use super::*;

#[test]
fn the_query_carries_every_filter_in_githubs_syntax() {
    let search = PullRequestSearch {
        state: "closed".into(),
        involvement: Some(Involvement::ReviewRequested("bob".into())),
        text: Some("fix  \"picker\" \"".into()),
        draft: Some("hide".into()),
        review: Some("changes-requested".into()),
        checks: Some("failing".into()),
        labels: vec![vec!["bug".into(), "ui".into()], vec!["p1".into()]],
        excluded_labels: vec!["wontfix".into()],
        author: Some("alice".into()),
    };
    assert_eq!(
        search_query("acme", "app", &search),
        "repo:acme/app is:pr is:closed is:unmerged review-requested:\"bob\" draft:false \
         review:changes_requested status:failure label:\"bug\",\"ui\" label:\"p1\" \
         -label:\"wontfix\" author:\"alice\" \"fix\" \"picker\" sort:updated-desc"
    );
}

#[test]
fn an_unfiltered_query_lists_every_pull_request() {
    let search = PullRequestSearch {
        state: "all".into(),
        ..Default::default()
    };
    assert_eq!(
        search_query("acme", "app", &search),
        "repo:acme/app is:pr sort:updated-desc"
    );
}

#[test]
fn a_search_response_becomes_a_page() {
    let data: SearchData = serde_json::from_value(serde_json::json!({
        "search": {
            "pageInfo": { "hasNextPage": true, "endCursor": "Y3Vy" },
            "nodes": [
                {
                    "number": 7, "title": "Move the picker", "url": "https://github.com/acme/app/pull/7",
                    "state": "MERGED", "isDraft": false, "mergeable": "CONFLICTING",
                    "additions": 3, "deletions": 1,
                    "createdAt": "2026-10-01T09:00:00Z", "updatedAt": "2026-10-02T09:00:00Z",
                    "headRefName": "feat/picker", "baseRefName": "main",
                    "reviewDecision": "CHANGES_REQUESTED",
                    "author": { "__typename": "Bot", "login": "renovate", "avatarUrl": null },
                    "labels": { "nodes": [{ "name": "ui", "color": "ff0000" }] },
                    "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "bob" } }, { "requestedReviewer": {} }] },
                    "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "ERROR" } } }] }
                },
                null,
                {}
            ]
        }
    }))
    .unwrap();
    assert_eq!(data.search.nodes.iter().flatten().count(), 2);
    let node = data
        .search
        .nodes
        .into_iter()
        .next()
        .unwrap()
        .unwrap()
        .into_listed();
    assert_eq!(node.state, "merged");
    assert_eq!(node.mergeability, "conflicting");
    assert_eq!(node.review_decision.as_deref(), Some("changes-requested"));
    assert_eq!(node.checks_state.as_deref(), Some("failing"));
    assert!(node.author_is_bot);
    assert_eq!(
        node.labels,
        vec![("ui".to_string(), Some("ff0000".to_string()))]
    );
    assert_eq!(node.requested_reviewers, vec!["bob".to_string()]);
}

#[test]
fn graphql_sits_beside_the_rest_root() {
    assert_eq!(
        super::super::api::graphql_url("https://api.github.com"),
        "https://api.github.com/graphql"
    );
    assert_eq!(
        super::super::api::graphql_url("https://ghe.example.com/api/v3/"),
        "https://ghe.example.com/api/graphql"
    );
}
