//! Table vs JSON output formatting.

/// Print any `Serialize` value as pretty JSON.
pub fn print_json_typed<T: serde::Serialize>(value: &T) {
    if let Ok(pretty) = serde_json::to_string_pretty(value) {
        println!("{pretty}");
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

    // Calculate column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() && cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    // Print header
    let header_line: Vec<String> = headers
        .iter()
        .zip(&widths)
        .map(|(h, w)| format!("{h:<width$}", width = *w))
        .collect();
    println!("{}", header_line.join("  "));

    let separator: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    println!("{}", separator.join("  "));

    // Print rows
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<width$}", width = *w))
            .collect();
        println!("{}", line.join("  "));
    }
}
