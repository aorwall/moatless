//! Expected text is the client's `serializeLegacyContextMessage` output for
//! the same input, so a drift from the client fails here.

use serde_json::json;

use super::*;

fn folded(text: &str, records: serde_json::Value) -> String {
    inline_message_text(text, Some(&json!({ "version": 1, "records": records })))
}

#[test]
fn a_terminal_chip_becomes_its_slug_and_a_numbered_block() {
    let records: Value = serde_json::from_str(r##"[{"version": 1, "kind": "terminal", "contextId": "term_a", "label": "Terminal 1 lines 2-3", "terminalId": "t1", "terminalLabel": "Terminal  1", "lineStart": 2, "lineEnd": 3, "text": "error: x\n  at y\n"}]"##).unwrap();
    assert_eq!(
        folded(
            "look at [Terminal 1 lines 2-3](t3-context://v1/terminal/term_a) please  \n",
            records
        ),
        "look at @terminal-1:2-3 please\n\n<terminal_context>\n- Terminal  1 lines 2-3:\n  2 | error: x\n  3 |   at y\n</terminal_context>"
    );
}

#[test]
fn a_referenced_review_comment_replaces_its_link_and_the_rest_trail() {
    let records: Value = serde_json::from_str(r##"[{"version": 1, "kind": "review-comment", "contextId": "rc_1", "label": "src/a.ts L4", "sectionId": "s1", "sectionTitle": "A \"quoted\" <title>", "filePath": "src/a.ts", "startIndex": 4, "endIndex": 6, "rangeLabel": "L4-L6", "text": "Rename this", "diff": "-a\n+```b```", "fenceLanguage": "ts"}, {"version": 1, "kind": "review-comment", "contextId": "rc_2", "label": "b.rs", "sectionId": "s2", "sectionTitle": "B", "filePath": "b.rs", "startIndex": 0, "endIndex": 0, "rangeLabel": "L1", "text": "No diff", "diff": ""}]"##).unwrap();
    assert_eq!(
        folded("fix [review](t3-context://v1/review-comment/rc_1)", records),
        "fix <review_comment sectionId=\"s1\" sectionTitle=\"A &quot;quoted&quot; &lt;title&gt;\" filePath=\"src/a.ts\" rangeLabel=\"L4-L6\" startIndex=\"4\" endIndex=\"6\">\nRename this\n\n````ts\n-a\n+```b```\n````\n</review_comment>\n\n<review_comment sectionId=\"s2\" sectionTitle=\"B\" filePath=\"b.rs\" rangeLabel=\"L1\" startIndex=\"0\" endIndex=\"0\">\nNo diff\n</review_comment>"
    );
}

#[test]
fn element_and_preview_records_trail_as_blocks() {
    let records: Value = serde_json::from_str(r##"[{"version": 1, "kind": "element", "contextId": "el_1", "label": "button", "pageUrl": "http://x/", "pageTitle": null, "tagName": "button", "selector": "#go", "htmlPreview": "<button>\n\nGo</button>", "componentName": null, "source": {"functionName": null, "fileName": "App.tsx", "lineNumber": 12, "columnNumber": null}, "styles": "color: red"}, {"version": 1, "kind": "preview-annotation", "contextId": "pa_1", "label": "note", "annotationId": "a1", "pageUrl": "", "pageTitle": "Home", "comment": "bigger", "targetSummary": "1 element", "styleChanges": ["font-size: 20px"], "elements": [{"pageUrl": "http://x/", "pageTitle": null, "tagName": "h1", "selector": null, "htmlPreview": "", "componentName": "Title", "source": null, "styles": ""}]}]"##).unwrap();
    assert_eq!(
        folded("style it", records),
        "style it\n\n<element_context>\n- <button>:\n  url: http://x/\n  selector: #go\n  source: App.tsx:12\n  html:\n    <button>\n\n    Go</button>\n  styles:\n    color: red\n</element_context>\n\n<preview_annotation>\nId: a1\nPage: Home\nComment: bigger\nTargets: 1 element\nRequested visual changes:\n- font-size: 20px\n<element_context>\n- <Title>:\n  url: http://x/\n</element_context>\n</preview_annotation>"
    );
}

#[test]
fn other_links_become_labels_and_unknown_ones_stay() {
    let records: Value = serde_json::from_str(r##"[{"version": 1, "kind": "image", "contextId": "img_1", "label": "shot.png", "attachmentId": "att1", "name": "shot.png", "mimeType": "image/png", "sizeBytes": 3}, {"version": 1, "kind": "thread", "contextId": "th_1", "label": "Fix the bug", "environmentId": "e", "threadId": "t", "title": "Fix the bug"}]"##).unwrap();
    assert_eq!(
        folded(
            "see ![shot](t3-context://v1/image/img_1) and [a [Thread](t3-context://v1/thread/th_1) and [gone](t3-context://v1/thread/missing) and [bad](t3-context://v1/thread/x y)",
            records
        ),
        "see shot.png and Fix the bug and [gone](t3-context://v1/thread/missing) and [bad](t3-context://v1/thread/x y)"
    );
}

#[test]
fn empty_records_only_trim_the_text() {
    let records: Value = serde_json::from_str(r##"[]"##).unwrap();
    assert_eq!(folded("plain  ", records), "plain");
}

#[test]
fn no_context_leaves_the_text_untouched() {
    assert_eq!(inline_message_text("hi  ", None), "hi  ");
    assert_eq!(inline_message_text("hi  ", Some(&Value::Null)), "hi  ");
}
