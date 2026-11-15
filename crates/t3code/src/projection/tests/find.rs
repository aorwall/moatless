use super::*;

#[test]
fn a_query_is_counted_in_the_text_a_reader_sees() {
    let segments = rendered_segments(
        "See [the docs](https://docs.example/docs) and `docs`.\n\n```\ndocs\n```",
        false,
    );
    assert_eq!(segments, vec!["See the docs and docs.", "docs\n"]);
}

/// `remark-breaks` renders each line of a user message as its own line, so a
/// phrase across one is not found, while an assistant's soft break joins.
#[test]
fn a_line_break_splits_a_user_message_and_joins_an_assistant_one() {
    assert_eq!(
        rendered_segments("fix the\nbuild", true),
        vec!["fix the", "build"]
    );
    assert_eq!(
        rendered_segments("fix the\nbuild", false),
        vec!["fix the build"]
    );
}

#[test]
fn occurrences_fold_case_sigma_and_curly_apostrophes() {
    assert_eq!(count_occurrences("Build, BUILD, build", "build"), 3);
    assert_eq!(count_occurrences("ΟΔΟΣ οδος", "οδοσ"), 2);
    assert_eq!(count_occurrences("don\u{2019}t", "don't"), 1);
    assert_eq!(count_occurrences("aaaa", "aa"), 2);
    assert_eq!(count_occurrences("text", ""), 0);
}

fn entry(id: i32, kind: MessageType, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        id,
        uuid: format!("m{id}"),
        message_type: kind,
        message_role: moatless_types::message::MessageRole::User,
        message_text: Some(text.to_string()),
        user_type: "human".to_string(),
        sandbox_wake: false,
        turn_number: Some(1),
        held: false,
        queue_order: None,
        timestamp: String::new(),
        created_at: None,
        external_adapter: None,
        external_source_name: None,
        external_subject_key: None,
        external_event_id: None,
        external_author: None,
        source_task_id: None,
        sent_by: None,
        file_contexts: None,
        child_outcome: None,
        harness: None,
        model: None,
        effort: None,
    }
}

/// What the timeline shows as no message row is not searched.
#[test]
fn only_message_rows_are_searched() {
    let mut queued = entry(1, MessageType::User, "x");
    queued.turn_number = None;
    let mut wake = entry(2, MessageType::User, "x");
    wake.sandbox_wake = true;
    let mut notice = entry(3, MessageType::User, "x");
    notice.child_outcome = Some("completed".into());
    let mut answer = entry(4, MessageType::User, "x");
    answer.user_type = TOOL_RESPONSE_USER_TYPE.to_string();
    let result = entry(5, MessageType::Result, "x");
    for hidden in [queued, wake, notice, answer, result] {
        assert_eq!(searchable_segments(&hidden), None, "{}", hidden.uuid);
    }
    assert!(searchable_segments(&entry(6, MessageType::Assistant, "x")).is_some());
}

fn find_over(texts: &[&str], find: &mut ThreadFind) -> Vec<Value> {
    texts
        .iter()
        .enumerate()
        .filter_map(|(index, text)| {
            let id = i32::try_from(index).unwrap_or(0) + 1;
            find.feed(&entry(id, MessageType::Assistant, text), 5)
        })
        .collect()
}

#[test]
fn a_start_resumes_from_its_occurrence_and_a_step_moves_on() {
    let start = FindStart {
        entry_id: "m2".into(),
        occurrence: 1,
    };
    let mut find = ThreadFind::new("task", "x", None, Some(1), Some(start));
    find_over(&["x", "x x", "x"], &mut find);
    let result = find.finish(5);
    assert_eq!(result["totalMatches"], 4);
    assert_eq!(result["activeIndex"], 3);
    assert_eq!(result["match"]["entryId"], "m3");
    assert_eq!(result["match"]["occurrence"], 0);
}

#[test]
fn an_index_past_the_end_selects_the_last_occurrence() {
    let mut find = ThreadFind::new("task", "x", Some(9), None, None);
    find_over(&["x", "x x"], &mut find);
    let result = find.finish(5);
    assert_eq!(result["activeIndex"], 2);
    assert_eq!(result["match"]["entryId"], "m2");
    assert_eq!(result["match"]["occurrence"], 1);
}

#[test]
fn no_occurrence_selects_nothing() {
    let mut find = ThreadFind::new("task", "zzz", None, None, None);
    find_over(&["x"], &mut find);
    let result = find.finish(5);
    assert_eq!(result["totalMatches"], 0);
    assert_eq!(result["match"], Value::Null);
    assert_eq!(result["navigation"], json!([]));
}

/// The early frame is the first occurrence at or after `start`, sent once.
#[test]
fn the_early_frame_is_the_first_occurrence_from_the_start() {
    let start = FindStart {
        entry_id: "m2".into(),
        occurrence: 0,
    };
    let mut find = ThreadFind::new("task", "x", None, None, Some(start));
    let early = find_over(&["x", "nothing", "x", "x"], &mut find);
    assert_eq!(early.len(), 1);
    assert_eq!(early[0]["match"]["entryId"], "m3");
    assert_eq!(early[0]["complete"], false);
}

/// Past the contract's cap, navigation is the window around the selection,
/// wrapping round the ends.
#[test]
fn navigation_is_a_window_around_the_selection() {
    let mut find = ThreadFind::new("task", "x", Some(0), None, None);
    let texts = vec!["x"; 20];
    find_over(&texts, &mut find);
    let result = find.finish(5);
    let navigation = result["navigation"].as_array().expect("navigation");
    assert_eq!(navigation.len(), NAVIGATION_MAX);
    assert_eq!(navigation[0]["entryId"], "m13");
    assert_eq!(navigation[8]["entryId"], "m1");
}
