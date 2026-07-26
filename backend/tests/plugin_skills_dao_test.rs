#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for the stored side of plugin skills.
//!
//! The point of storing skills at all is that they outlive the process that
//! read them out of git, so these run against a real database rather than the
//! in-memory stub used by the sync unit tests.

use moatless_vibe_backend::plugin::dao::{PgPluginRepository, PluginRepository};
use moatless_vibe_backend::plugin::models::{Plugin, PluginSkill};
use sqlx::PgPool;

fn plugin(id: &str) -> Plugin {
    Plugin {
        id: id.to_string(),
        name: format!("plugin-{id}"),
        git_url: format!("https://github.com/acme/{id}"),
        synced_version: None,
        synced_at: None,
        created_at: None,
        updated_at: None,
    }
}

fn skill(plugin_id: &str, name: &str) -> PluginSkill {
    PluginSkill {
        plugin_id: plugin_id.to_string(),
        name: name.to_string(),
        description: format!("does {name}"),
    }
}

#[sqlx::test]
async fn stored_skills_outlive_the_reader_that_fetched_them(pool: PgPool) {
    let dao = PgPluginRepository(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");

    dao.replace_skills(
        "p1",
        Some("sha-1".to_string()),
        &[skill("p1", "dev-loop"), skill("p1", "code-review")],
    )
    .await
    .expect("store skills");

    // A second DAO over the same database stands in for the next process:
    // nothing is carried over in memory.
    let reader = PgPluginRepository(pool);
    let stored = reader.list_skills("p1").await.expect("read skills");

    assert_eq!(
        stored.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["code-review", "dev-loop"],
        "skills come back by name, from storage rather than from git"
    );
    assert_eq!(
        reader
            .find_by_id("p1")
            .await
            .expect("find")
            .expect("row")
            .synced_version,
        Some("sha-1".to_string()),
        "the version is what a later sync compares against to avoid a fetch"
    );
}

#[sqlx::test]
async fn a_resync_replaces_the_previous_snapshot_rather_than_adding_to_it(pool: PgPool) {
    let dao = PgPluginRepository(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");

    dao.replace_skills(
        "p1",
        Some("sha-1".to_string()),
        &[skill("p1", "dev-loop"), skill("p1", "retired")],
    )
    .await
    .expect("first sync");

    dao.replace_skills(
        "p1",
        Some("sha-2".to_string()),
        &[skill("p1", "dev-loop"), skill("p1", "added")],
    )
    .await
    .expect("second sync");

    let stored = dao.list_skills("p1").await.expect("read skills");
    assert_eq!(
        stored.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["added", "dev-loop"],
        "a skill deleted upstream must disappear here too, or it stays \
         selectable forever"
    );
}

#[sqlx::test]
async fn deleting_a_plugin_takes_its_skills_with_it(pool: PgPool) {
    let dao = PgPluginRepository(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    dao.replace_skills("p1", Some("sha-1".to_string()), &[skill("p1", "dev-loop")])
        .await
        .expect("store skills");

    assert!(dao.delete("p1").await.expect("delete"));

    assert!(
        dao.list_skills("p1").await.expect("read skills").is_empty(),
        "orphaned skill rows would outlive the plugin they describe"
    );
}

#[sqlx::test]
async fn skills_are_scoped_to_their_own_plugin(pool: PgPool) {
    let dao = PgPluginRepository(pool);
    dao.insert(&plugin("p1")).await.expect("insert p1");
    dao.insert(&plugin("p2")).await.expect("insert p2");

    dao.replace_skills("p1", Some("a".to_string()), &[skill("p1", "shared-name")])
        .await
        .expect("store p1 skills");
    dao.replace_skills("p2", Some("b".to_string()), &[skill("p2", "shared-name")])
        .await
        .expect("store p2 skills");

    // Two plugins may legitimately hold a skill of the same name; the primary
    // key is (plugin_id, name), so neither sync clobbers the other.
    assert_eq!(dao.list_skills("p1").await.expect("p1").len(), 1);
    assert_eq!(dao.list_skills("p2").await.expect("p2").len(), 1);

    dao.replace_skills("p1", Some("c".to_string()), &[])
        .await
        .expect("empty p1");
    assert!(dao.list_skills("p1").await.expect("p1").is_empty());
    assert_eq!(
        dao.list_skills("p2").await.expect("p2").len(),
        1,
        "one plugin's resync must not touch another's skills"
    );
}
