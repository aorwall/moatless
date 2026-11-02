//! Transfer a fork's completed conversation back to its parent.

use super::error::TaskError;
use super::fork::TaskForkService;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::TaskRelationType;

impl TaskForkService {
    /// Queue completed fork turns as context, without starting the parent's agent.
    pub async fn merge_back(
        &self,
        viewer: &ScopeContext,
        source: &str,
        target: &str,
        through: i32,
    ) -> Result<(), TaskError> {
        let source_task = self
            .tasks
            .find_by_id(source, viewer)
            .await?
            .filter(|task| viewer.can_read(&task.scoped_resource()))
            .ok_or_else(|| TaskError::NotFound(source.into()))?;
        let target_task = self
            .tasks
            .find_by_id(target, viewer)
            .await?
            .filter(|task| viewer.can_write(&task.scoped_resource(), Some(task.visibility)))
            .ok_or_else(|| TaskError::NotFound(target.into()))?;
        let parent = self
            .relations
            .find_parent(&source_task.task_id)
            .await?
            .filter(|parent| {
                parent.relation_type == TaskRelationType::ForkedFrom
                    && parent.to_task_id == target_task.task_id
            })
            .ok_or_else(|| {
                TaskError::BadRequest("The source must be a fork of the target thread.".into())
            })?;
        let after = parent
            .context
            .get("at_turn")
            .and_then(serde_json::Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(0);
        let point = self.turns.fork_point(source, Some(through)).await?;
        if through <= after
            || point.turn != Some(through)
            || point.open_turn.is_some_and(|open| through >= open)
        {
            return Err(TaskError::Conflict(
                "Choose a completed turn belonging to the fork.".into(),
            ));
        }
        let items = self
            .messages
            .merge_back_context(source, after, through)
            .await?;
        let summary = summary(source, through, &items);
        if !self
            .merge_backs
            .queue(source, target, through, &summary)
            .await?
        {
            return Err(TaskError::Conflict(
                "The parent must be open and have no active or queued turns.".into(),
            ));
        }
        Ok(())
    }
}

fn summary(source: &str, through: i32, items: &[(String, String)]) -> String {
    let mut text = format!(
        "Merge-back context from forked conversation.\n\nSource: `{source}`, through turn {through}.\nRun `moat messages list {source} --full` for the full conversation.\n\n"
    );
    let mut remaining = 16_000usize.saturating_sub(text.chars().count() + 30);
    let mut selected = Vec::new();
    for (author, content) in items {
        let content = content.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut item: String = format!("- {author}: {content}").chars().take(799).collect();
        item.push('\n');
        let length = item.chars().count();
        if length > remaining {
            text.push_str("- Earlier items truncated.\n");
            break;
        }
        remaining -= length;
        selected.push(item);
    }
    for item in selected.iter().rev() {
        text.push_str(item);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn newest_window_is_rendered_chronologically_with_bounded_unicode_items() {
        let items: Vec<_> = (0..30)
            .rev()
            .map(|n| ("agent".into(), format!("item-{n:02} {}", "界".repeat(900))))
            .collect();
        let text = summary("source", 30, &items);
        assert!(text.chars().count() <= 16_000);
        let bullets: Vec<_> = text.lines().filter(|line| line.starts_with("- ")).collect();
        assert_eq!(bullets[0], "- Earlier items truncated.");
        assert!(bullets[1..].windows(2).all(|pair| pair[0] < pair[1]));
        assert!(bullets.last().unwrap().starts_with("- agent: item-29 "));
        assert!(bullets.iter().all(|line| line.chars().count() < 800));
        assert!(!text.contains("item-00"));
        let text = summary(
            "source",
            2,
            &[
                ("agent".into(), "reply".into()),
                ("user".into(), "question".into()),
            ],
        );
        assert!(text.ends_with("- user: question\n- agent: reply\n"));
        assert!(!text.contains("truncated"));
    }
}
