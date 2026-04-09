//! Miscellaneous commands: commands list, tags list.

use moatless_api_schemas::command::CommandResponse;
use moatless_api_schemas::tag::TagResponse;

use crate::output;

pub fn format_commands(commands: &[CommandResponse]) {
    let rows: Vec<Vec<String>> = commands
        .iter()
        .map(|c| vec![c.name.clone(), c.description.chars().take(60).collect()])
        .collect();

    output::print_table(&["NAME", "DESCRIPTION"], &rows);
}

pub fn format_tags(tags: &[TagResponse]) {
    let rows: Vec<Vec<String>> = tags
        .iter()
        .map(|t| vec![t.tag_id.clone(), t.name.clone()])
        .collect();

    output::print_table(&["TAG_ID", "NAME"], &rows);
}
