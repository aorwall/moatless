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

use moatless_vibe_backend::plugin::dao::{PgPluginDao, PluginDao};
use moatless_vibe_backend::plugin::models::{ActivationRow, Plugin, PluginSkill};
use sqlx::PgPool;
use uuid::Uuid;

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
    let dao = PgPluginDao(pool.clone());
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
    let reader = PgPluginDao(pool);
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
    let dao = PgPluginDao(pool);
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
    let dao = PgPluginDao(pool);
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
    let dao = PgPluginDao(pool);
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

// ---- Activation records ----

/// A real user row: an activation record's `user_id` is a foreign key, so a
/// personal record cannot be written for somebody who does not exist.
async fn user(pool: &PgPool, login: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, login, provider, provider_user_id, created_at, last_login_at) \
         VALUES ($1, $2, 'test', $2, now(), now())",
    )
    .bind(id)
    .bind(login)
    .execute(pool)
    .await
    .expect("insert user");
    id
}

#[sqlx::test]
async fn one_plugin_holds_at_most_one_global_record(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");

    let on = ActivationRow {
        plugin_id: "p1".to_string(),
        skill_name: None,
        user_id: None,
        enabled: true,
    };
    dao.set_activation(&on).await.expect("turn on");
    dao.set_activation(&ActivationRow {
        enabled: false,
        ..on.clone()
    })
    .await
    .expect("turn off");

    let rows = dao.list_activations_for(None).await.expect("list");
    assert_eq!(
        rows,
        vec![ActivationRow {
            enabled: false,
            ..on
        }],
        "writing the global default twice replaces it — NULL user_id must not \
         create a second row"
    );
}

#[sqlx::test]
async fn resolution_sees_global_records_plus_only_its_own(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let alice_id = user(&pool, "alice").await;
    let bob_id = user(&pool, "bob").await;

    for row in [
        ActivationRow {
            plugin_id: "p1".into(),
            skill_name: None,
            user_id: None,
            enabled: true,
        },
        ActivationRow {
            plugin_id: "p1".into(),
            skill_name: None,
            user_id: Some(alice_id),
            enabled: false,
        },
        ActivationRow {
            plugin_id: "p1".into(),
            skill_name: None,
            user_id: Some(bob_id),
            enabled: false,
        },
    ] {
        dao.set_activation(&row).await.expect("set");
    }

    let alice = dao.list_activations_for(Some(alice_id)).await.expect("list");
    assert_eq!(alice.len(), 2, "the global default plus alice's own");
    assert!(
        !alice.iter().any(|r| r.user_id == Some(bob_id)),
        "one person's choices must never enter another's resolution"
    );
}

#[sqlx::test]
async fn clearing_a_record_is_not_the_same_as_turning_it_off(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let alice_id = user(&pool, "alice").await;

    dao.set_activation(&ActivationRow {
        plugin_id: "p1".into(),
        skill_name: None,
        user_id: Some(alice_id),
        enabled: false,
    })
    .await
    .expect("set");

    assert!(dao
        .clear_activation("p1", None, Some(alice_id))
        .await
        .expect("clear"));

    assert!(
        dao.list_activations_for(Some(alice_id))
            .await
            .expect("list")
            .is_empty(),
        "clearing hands alice back to the global default rather than pinning \
         them to off"
    );
    assert!(
        !dao.clear_activation("p1", None, Some(alice_id))
            .await
            .expect("clear again"),
        "clearing what is not there reports so, rather than pretending"
    );
}

#[sqlx::test]
async fn a_skill_record_and_a_plugin_record_coexist(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");

    dao.set_activation(&ActivationRow {
        plugin_id: "p1".into(),
        skill_name: None,
        user_id: None,
        enabled: true,
    })
    .await
    .expect("plugin record");
    dao.set_activation(&ActivationRow {
        plugin_id: "p1".into(),
        skill_name: Some("noisy".into()),
        user_id: None,
        enabled: false,
    })
    .await
    .expect("skill record");

    assert_eq!(
        dao.list_activations_for(None).await.expect("list").len(),
        2,
        "\"the whole plugin, except this skill\" needs both records to survive"
    );

    assert!(dao
        .clear_activation("p1", Some("noisy".to_string()), None)
        .await
        .expect("clear the skill record"));
    assert_eq!(
        dao.list_activations_for(None).await.expect("list").len(),
        1,
        "clearing the skill record must not take the plugin record with it"
    );
}

#[sqlx::test]
async fn deleting_a_plugin_takes_its_activations_with_it(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    dao.set_activation(&ActivationRow {
        plugin_id: "p1".into(),
        skill_name: None,
        user_id: None,
        enabled: true,
    })
    .await
    .expect("set");

    assert!(dao.delete("p1").await.expect("delete"));
    assert!(
        dao.list_activations_for(None)
            .await
            .expect("list")
            .is_empty(),
        "a removed plugin must stop being delivered, records and all"
    );
}
