//! Table vs JSON output formatting.

/// Print any `Serialize` value as pretty JSON.
pub fn print_json_typed<T: serde::Serialize>(value: &T) {
    if let Ok(pretty) = serde_json::to_string_pretty(value) {
        println!("{pretty}");
    }
}

/// Print `value` as pretty JSON when `json` is set, otherwise run `human`.
pub fn emit<T: serde::Serialize>(json: bool, value: &T, human: impl FnOnce()) {
    if json {
        print_json_typed(value);
    } else {
        human();
    }
}

/// Print a simple table from a list of rows.
///
/// Each row is a Vec of column values. First row is headers.
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        println!("No results.");
        return;
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() && cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    let header_line: Vec<String> = headers
        .iter()
        .zip(&widths)
        .map(|(h, w)| format!("{h:<width$}", width = *w))
        .collect();
    println!("{}", header_line.join("  "));

    let separator: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    println!("{}", separator.join("  "));

    for row in rows {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<width$}", width = *w))
            .collect();
        println!("{}", line.join("  "));
    }
}

/// Cut `text` to at most `width` characters, ending in `…` when anything was
/// dropped. The `…` takes the last character's place rather than following it,
/// so the result never exceeds `width`.
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}
