use super::*;

fn query() -> IssueListQuery {
    IssueListQuery {
        scope: "sp".to_string(),
        state: IssueStateFilter::Open,
        involvement: IssueInvolvement::All,
        query: None,
        labels: Vec::new(),
        assignee: None,
        sort: IssueSort::Updated,
        limit: 50,
        after: None,
    }
}

#[test]
fn an_identifier_is_a_team_key_and_a_number() {
    assert_eq!(parse_identifier(" sp-12 ").as_deref(), Some("SP-12"));
    assert_eq!(parse_identifier("SP-012"), None);
    assert_eq!(parse_identifier("SP"), None);
    assert_eq!(parse_identifier("12-SP"), None);
}

#[test]
fn a_query_carries_only_the_fragments_it_spreads() {
    let list = with_fragments(LIST_QUERY);
    for used in [
        "fragment I ",
        "fragment U ",
        "fragment B ",
        "fragment S ",
        "fragment L ",
        "fragment P ",
    ] {
        assert!(list.contains(used), "{used} missing");
    }
    assert!(!list.contains("fragment C "));

    let user = with_fragments(USER_QUERY);
    assert!(user.contains("fragment U "));
    assert!(!user.contains("fragment I "));
}

#[test]
fn an_open_listing_filters_on_the_team_and_excludes_closed_state_types() {
    let filter = list_filter(&query(), None).unwrap();
    assert_eq!(filter["team"], json!({ "key": { "eq": "SP" } }));
    assert_eq!(
        filter["state"],
        json!({ "type": { "nin": CLOSED_STATE_TYPES } })
    );
    assert!(filter.get("assignee").is_none());
}

#[test]
fn an_involvement_needs_the_viewers_linear_user() {
    let assigned = IssueListQuery {
        involvement: IssueInvolvement::Assigned,
        ..query()
    };
    assert!(matches!(
        list_filter(&assigned, None),
        Err(IssueTrackerError::Refused(_))
    ));
    let filter = list_filter(&assigned, Some("user-9")).unwrap();
    assert_eq!(filter["assignee"], json!({ "id": { "eq": "user-9" } }));
}

#[test]
fn a_text_query_that_names_a_number_also_matches_it() {
    let searched = IssueListQuery {
        query: Some("SP-12".to_string()),
        ..query()
    };
    let filter = list_filter(&searched, None).unwrap();
    assert_eq!(filter["or"][2], json!({ "number": { "eq": 12 } }));
}

fn linear_state(id: &str, kind: &str) -> LinearState {
    LinearState {
        id: id.to_string(),
        name: id.to_string(),
        kind: kind.to_string(),
        position: None,
    }
}

#[test]
fn a_move_into_a_closed_state_is_a_close_event() {
    let entry = LinearHistoryEntry {
        id: "h1".to_string(),
        created_at: "2026-11-01T09:00:00Z".to_string(),
        actor: None,
        bot_actor: None,
        from_state: Some(linear_state("Todo", "unstarted")),
        to_state: Some(linear_state("Done", "completed")),
        from_assignee: None,
        to_assignee: None,
        added_labels: None,
        removed_labels: None,
        from_title: None,
        to_title: None,
    };
    let events = events(&[entry]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "closed");
    assert_eq!(events[0].detail.as_deref(), Some("Done"));
}

#[test]
fn only_a_github_pull_request_attachment_is_a_linked_pull_request() {
    let attachment = |url: &str, status: &str| LinearAttachment {
        url: url.to_string(),
        title: String::new(),
        metadata: json!({ "status": status }),
    };
    let linked = linked_pull_requests(&[
        attachment("https://github.com/acme/app/pull/7", "merged"),
        attachment("https://github.com/acme/app/issues/8", "open"),
        attachment("https://github.com/acme/app/pull/7", "merged"),
    ]);
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].repository, "acme/app");
    assert_eq!(linked[0].number, 7);
    assert_eq!(linked[0].state, "merged");
}
