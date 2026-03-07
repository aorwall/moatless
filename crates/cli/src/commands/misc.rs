//! Miscellaneous commands: commands list, tags list.

use crate::output;

pub fn format_commands(resp: &serde_json::Value) {
    let items = resp.as_array().cloned().unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|c| {
            vec![
                c.get("id").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
                c.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                c.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .chars()
                    .take(60)
                    .collect(),
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "DESCRIPTION"], &rows);
}

pub fn format_tags(resp: &serde_json::Value) {
    let items = resp.as_array().cloned().unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|t| {
            vec![
                t.get("tagId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                t.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
            ]
        })
        .collect();

    output::print_table(&["TAG_ID", "NAME"], &rows);
}
