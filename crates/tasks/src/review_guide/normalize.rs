//! Holds a submitted guide to the change it describes.
//!
//! An agent's line numbers are suggestions: excerpts are clamped to changed lines, notes must sit
//! inside what is left, and a changed file no stop placed lands in a closing
//! skim stop.

use std::collections::HashSet;

use moatless_api_schemas::review_guide::{
    ReviewGuideChange, ReviewGuideExcerpt, ReviewGuideFile, ReviewGuideSide, ReviewGuideStop,
};

/// Longest excerpt kept. The reader wants the lines that carry the idea, not
/// the whole hunk.
const MAX_EXCERPT_LINES: u32 = 25;

/// A guide held to its change, with what had to be dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedGuide {
    pub stops: Vec<ReviewGuideStop>,
    pub submitted_excerpts: usize,
    pub dropped_excerpts: usize,
    pub dropped_notes: usize,
    pub unplaced_files: usize,
}

impl NormalizedGuide {
    pub fn excerpts(&self) -> usize {
        self.stops.iter().map(|stop| stop.excerpts.len()).sum()
    }
}

pub fn normalize(stops: Vec<ReviewGuideStop>, change: &ReviewGuideChange) -> NormalizedGuide {
    let changed: HashSet<&str> = change.files.iter().map(String::as_str).collect();
    let mut submitted_excerpts = 0;
    let mut dropped_excerpts = 0;
    let mut dropped_notes = 0;

    let shaped: Vec<ReviewGuideStop> = stops
        .into_iter()
        .filter(|stop| !stop.title.trim().is_empty())
        .map(|stop| {
            submitted_excerpts += stop.excerpts.len();
            let clamped: Vec<ReviewGuideExcerpt> = stop
                .excerpts
                .into_iter()
                .filter_map(|excerpt| {
                    let clamped = changed
                        .contains(excerpt.path.as_str())
                        .then(|| clamp_excerpt(excerpt, change))
                        .flatten();
                    if clamped.is_none() {
                        dropped_excerpts += 1;
                    }
                    clamped
                })
                .collect();
            let excerpts = merge_excerpts(clamped);
            let notes: Vec<_> = stop
                .notes
                .into_iter()
                .filter(|note| {
                    let kept = !note.title.trim().is_empty()
                        && excerpts.iter().any(|excerpt| {
                            excerpt.path == note.path
                                && excerpt.start <= note.line
                                && note.line <= excerpt.end
                        });
                    if !kept {
                        dropped_notes += 1;
                    }
                    kept
                })
                .collect();
            ReviewGuideStop {
                title: stop.title,
                summary: stop.summary,
                attention: stop
                    .attention
                    .filter(|attention| !attention.note.trim().is_empty()),
                excerpts,
                files: stop
                    .files
                    .into_iter()
                    .filter(|file| changed.contains(file.path.as_str()))
                    .map(|file| ReviewGuideFile {
                        note: file.note.filter(|note| !note.trim().is_empty()),
                        path: file.path,
                    })
                    .collect(),
                notes,
                questions: stop
                    .questions
                    .into_iter()
                    .filter(|question| !question.trim().is_empty())
                    .collect(),
            }
        })
        .collect();
    // A file excerpted anywhere needs no files entry; one never excerpted is
    // listed once, by the first stop that names it.
    let mut placed: HashSet<String> = shaped
        .iter()
        .flat_map(|stop| stop.excerpts.iter().map(|excerpt| excerpt.path.clone()))
        .collect();
    let mut stops: Vec<ReviewGuideStop> = shaped
        .into_iter()
        .map(|mut stop| {
            stop.files.retain(|file| placed.insert(file.path.clone()));
            stop
        })
        .collect();
    let unplaced: Vec<ReviewGuideFile> = change
        .files
        .iter()
        .filter(|path| !placed.contains(path.as_str()))
        .map(|path| ReviewGuideFile {
            path: path.clone(),
            note: None,
        })
        .collect();
    let unplaced_files = unplaced.len();
    if !unplaced.is_empty() {
        stops.push(ReviewGuideStop {
            title: "Other files".to_string(),
            summary: "Changed files the tour does not otherwise cover.".to_string(),
            attention: None,
            excerpts: Vec::new(),
            files: unplaced,
            notes: Vec::new(),
            questions: Vec::new(),
        });
    }

    NormalizedGuide {
        stops,
        submitted_excerpts,
        dropped_excerpts,
        dropped_notes,
        unplaced_files,
    }
}

/// Narrows an excerpt to the changed lines it covers, from the first changed
/// line it overlaps to the last. `None` when it covers none: the agent pointed
/// where the change is not.
fn clamp_excerpt(
    excerpt: ReviewGuideExcerpt,
    change: &ReviewGuideChange,
) -> Option<ReviewGuideExcerpt> {
    let start = excerpt.start.min(excerpt.end);
    let end = excerpt.start.max(excerpt.end);
    let hunks = change.hunks.get(&excerpt.path)?;
    let ranges = match excerpt.side {
        ReviewGuideSide::New => &hunks.new,
        ReviewGuideSide::Old => &hunks.old,
    };
    let mut overlapping = ranges
        .iter()
        .filter(|hunk| hunk.start <= end && start <= hunk.end);
    let first = overlapping.next()?;
    let last = overlapping.next_back().unwrap_or(first);
    Some(ReviewGuideExcerpt {
        start: start.max(first.start),
        end: end.min(last.end),
        ..excerpt
    })
}

/// Joins a stop's excerpts that overlap or touch in one file, then caps each
/// one's length.
fn merge_excerpts(excerpts: Vec<ReviewGuideExcerpt>) -> Vec<ReviewGuideExcerpt> {
    let mut merged: Vec<ReviewGuideExcerpt> = Vec::new();
    for excerpt in excerpts {
        let touching = merged.iter_mut().find(|other| {
            other.path == excerpt.path
                && other.side == excerpt.side
                && other.start <= excerpt.end.saturating_add(1)
                && excerpt.start <= other.end.saturating_add(1)
        });
        match touching {
            Some(other) => {
                other.start = other.start.min(excerpt.start);
                other.end = other.end.max(excerpt.end);
                if other.caption.is_empty() {
                    other.caption = excerpt.caption;
                }
            }
            None => merged.push(excerpt),
        }
    }
    for excerpt in &mut merged {
        excerpt.end = excerpt
            .end
            .min(excerpt.start.saturating_add(MAX_EXCERPT_LINES - 1));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_api_schemas::review_guide::{
        ReviewGuideAttention, ReviewGuideAttentionKind, ReviewGuideFileHunks, ReviewGuideLineRange,
        ReviewGuideNote,
    };

    fn range(start: u32, end: u32) -> ReviewGuideLineRange {
        ReviewGuideLineRange { start, end }
    }

    fn change() -> ReviewGuideChange {
        let mut change = ReviewGuideChange {
            files: vec!["src/a.rs".into(), "src/b.rs".into(), "Cargo.lock".into()],
            ..Default::default()
        };
        change.hunks.insert(
            "src/a.rs".into(),
            ReviewGuideFileHunks {
                new: vec![range(10, 20), range(40, 45)],
                old: vec![range(8, 9)],
            },
        );
        change.hunks.insert(
            "src/b.rs".into(),
            ReviewGuideFileHunks {
                new: vec![range(1, 100)],
                old: vec![],
            },
        );
        change
    }

    fn excerpt(path: &str, start: u32, end: u32) -> ReviewGuideExcerpt {
        ReviewGuideExcerpt {
            path: path.into(),
            side: ReviewGuideSide::New,
            start,
            end,
            caption: String::new(),
        }
    }

    fn stop(title: &str, excerpts: Vec<ReviewGuideExcerpt>) -> ReviewGuideStop {
        ReviewGuideStop {
            title: title.into(),
            summary: "summary".into(),
            attention: None,
            excerpts,
            files: vec![],
            notes: vec![],
            questions: vec![],
        }
    }

    #[test]
    fn excerpts_are_clamped_to_the_changed_lines_they_overlap() {
        let guide = normalize(
            vec![stop("one", vec![excerpt("src/a.rs", 5, 42)])],
            &change(),
        );
        assert_eq!(guide.stops[0].excerpts, vec![excerpt("src/a.rs", 10, 34)]);
    }

    #[test]
    fn an_excerpt_outside_the_change_is_dropped_and_counted() {
        let guide = normalize(
            vec![stop(
                "one",
                vec![excerpt("src/a.rs", 25, 30), excerpt("src/other.rs", 1, 3)],
            )],
            &change(),
        );
        assert!(guide.stops[0].excerpts.is_empty());
        assert_eq!(guide.submitted_excerpts, 2);
        assert_eq!(guide.dropped_excerpts, 2);
    }

    #[test]
    fn old_side_excerpts_count_against_removed_lines() {
        let mut old = excerpt("src/a.rs", 1, 9);
        old.side = ReviewGuideSide::Old;
        let guide = normalize(vec![stop("one", vec![old])], &change());
        assert_eq!(
            (
                guide.stops[0].excerpts[0].start,
                guide.stops[0].excerpts[0].end
            ),
            (8, 9)
        );
    }

    #[test]
    fn touching_excerpts_merge_and_long_ones_are_capped() {
        let guide = normalize(
            vec![stop(
                "one",
                vec![excerpt("src/b.rs", 1, 10), excerpt("src/b.rs", 11, 60)],
            )],
            &change(),
        );
        assert_eq!(guide.stops[0].excerpts, vec![excerpt("src/b.rs", 1, 25)]);
        assert_eq!(guide.dropped_excerpts, 0);
    }

    #[test]
    fn notes_outside_kept_excerpts_and_blank_fields_are_dropped() {
        let mut first = stop("one", vec![excerpt("src/a.rs", 10, 15)]);
        first.attention = Some(ReviewGuideAttention {
            kind: ReviewGuideAttentionKind::Risk,
            note: "  ".into(),
        });
        first.questions = vec!["Why?".into(), " ".into()];
        first.notes = vec![
            ReviewGuideNote {
                path: "src/a.rs".into(),
                line: 12,
                title: "kept".into(),
                body: String::new(),
            },
            ReviewGuideNote {
                path: "src/a.rs".into(),
                line: 40,
                title: "outside".into(),
                body: String::new(),
            },
        ];
        let guide = normalize(vec![first, stop("  ", vec![])], &change());
        let shaped = &guide.stops[0];
        assert_eq!(shaped.attention, None);
        assert_eq!(shaped.questions, vec!["Why?".to_string()]);
        assert_eq!(shaped.notes.len(), 1);
        assert_eq!(guide.dropped_notes, 1);
        // The blank-titled stop is gone; the closing stop takes the rest.
        assert_eq!(guide.stops.len(), 2);
        assert_eq!(guide.stops[1].title, "Other files");
    }

    #[test]
    fn every_changed_file_is_placed_exactly_once() {
        let mut first = stop("one", vec![excerpt("src/a.rs", 10, 12)]);
        first.files = vec![
            ReviewGuideFile {
                path: "src/a.rs".into(),
                note: Some("excerpted already".into()),
            },
            ReviewGuideFile {
                path: "Cargo.lock".into(),
                note: Some(" ".into()),
            },
            ReviewGuideFile {
                path: "not/changed.rs".into(),
                note: None,
            },
        ];
        let mut second = stop("two", vec![]);
        second.files = vec![ReviewGuideFile {
            path: "Cargo.lock".into(),
            note: Some("again".into()),
        }];
        let guide = normalize(vec![first, second], &change());
        assert_eq!(
            guide.stops[0].files,
            vec![ReviewGuideFile {
                path: "Cargo.lock".into(),
                note: None
            }]
        );
        assert!(guide.stops[1].files.is_empty());
        let skim = guide.stops.last().unwrap();
        assert_eq!(skim.files.len(), 1);
        assert_eq!(skim.files[0].path, "src/b.rs");
        assert_eq!(guide.unplaced_files, 1);
    }
}
