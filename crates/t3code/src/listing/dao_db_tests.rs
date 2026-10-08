use super::changes_after;
use moatless_core::change_feed::listing::{current_position, record_change};
use moatless_tasks::task::test_support::{make_task_row, seed_task};
use sqlx::PgPool;

const OWNER: &str = "11111111-1111-1111-1111-111111111111";

/// A child's change is read as its parent's too, at the next position: the
/// shell stream drops an event whose sequence repeats. A peer is not a parent.
#[sqlx::test(migrations = "../core/migrations")]
async fn a_childs_change_is_read_for_its_parent(pool: PgPool) {
    for task_id in ["parent", "child", "peer"] {
        seed_task(&pool, &make_task_row(task_id, OWNER))
            .await
            .expect("seed task");
    }
    sqlx::query(
        "INSERT INTO task_relations (id, from_task_id, to_task_id, relation_type, context)
         VALUES ('child->parent', 'child', 'parent', 'created_by', '{}'),
                ('child->peer', 'child', 'peer', 'related_to', '{}')",
    )
    .execute(&pool)
    .await
    .expect("seed relations");
    let start = current_position(&pool).await.expect("head");

    let mut tx = pool.begin().await.expect("begin");
    let position = record_change(&mut tx, "child").await.expect("record");
    tx.commit().await.expect("commit");
    assert_eq!(current_position(&pool).await.expect("head"), position + 1);

    let read: Vec<(i64, String)> = changes_after(&pool, start, 1000)
        .await
        .expect("changes")
        .into_iter()
        .map(|change| (change.position, change.task_id))
        .collect();
    assert_eq!(
        read,
        vec![
            (position, "child".to_string()),
            (position + 1, "parent".to_string())
        ]
    );
}
