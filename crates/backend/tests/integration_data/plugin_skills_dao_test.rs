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

use moatless_backend::plugin::dao::{PgPluginDao, PluginDao};
use moatless_backend::plugin::models::{ActivationRow, ActivationWrite, Plugin, PluginSkill};
use sqlx::PgPool;
use uuid::Uuid;

/// A write of one record with both flags left unsaid, which is what all but the
/// flag tests below are about.
fn write(
    skill_name: Option<&str>,
    user_id: Option<Uuid>,
    workspace_id: Option<&str>,
    enabled: bool,
) -> ActivationWrite {
    ActivationWrite {
        plugin_id: "p1".to_string(),
        skill_name: skill_name.map(str::to_string),
        user_id,
        workspace_id: workspace_id.map(str::to_string),
        enabled,
        ..Default::default()
    }
}

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
        &[skill("p1", "build-check"), skill("p1", "code-review")],
    )
    .await
    .expect("store skills");

    // A second DAO over the same database stands in for the next process:
    // nothing is carried over in memory.
    let reader = PgPluginDao(pool);
    let stored = reader.list_skills("p1").await.expect("read skills");

    assert_eq!(
        stored.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["build-check", "code-review"],
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
        &[skill("p1", "build-check"), skill("p1", "retired")],
    )
    .await
    .expect("first sync");

    dao.replace_skills(
        "p1",
        Some("sha-2".to_string()),
        &[skill("p1", "build-check"), skill("p1", "added")],
    )
    .await
    .expect("second sync");

    let stored = dao.list_skills("p1").await.expect("read skills");
    assert_eq!(
        stored.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["added", "build-check"],
        "a skill deleted upstream must disappear here too, or it stays \
         selectable forever"
    );
}

#[sqlx::test]
async fn deleting_a_plugin_takes_its_skills_with_it(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    dao.replace_skills(
        "p1",
        Some("sha-1".to_string()),
        &[skill("p1", "build-check")],
    )
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

    dao.set_activation(&write(None, None, None, true))
        .await
        .expect("turn on");
    dao.set_activation(&write(None, None, None, false))
        .await
        .expect("turn off");

    let rows = dao.list_activations_for(None, None).await.expect("list");
    assert_eq!(
        rows,
        vec![ActivationRow {
            plugin_id: "p1".to_string(),
            skill_name: None,
            user_id: None,
            workspace_id: None,
            enabled: false,
            always: false,
            frontload: false,
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
        write(None, None, None, true),
        write(None, Some(alice_id), None, false),
        write(None, Some(bob_id), None, false),
    ] {
        dao.set_activation(&row).await.expect("set");
    }

    let alice = dao
        .list_activations_for(Some(alice_id), None)
        .await
        .expect("list");
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

    dao.set_activation(&write(None, Some(alice_id), None, false))
        .await
        .expect("set");

    assert!(
        dao.clear_activation("p1", None, Some(alice_id), None)
            .await
            .expect("clear")
    );

    assert!(
        dao.list_activations_for(Some(alice_id), None)
            .await
            .expect("list")
            .is_empty(),
        "clearing hands alice back to the global default rather than pinning \
         them to off"
    );
    assert!(
        !dao.clear_activation("p1", None, Some(alice_id), None)
            .await
            .expect("clear again"),
        "clearing what is not there reports so, rather than pretending"
    );
}

#[sqlx::test]
async fn a_skill_record_and_a_plugin_record_coexist(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");

    dao.set_activation(&write(None, None, None, true))
        .await
        .expect("plugin record");
    dao.set_activation(&write(Some("noisy"), None, None, false))
        .await
        .expect("skill record");

    assert_eq!(
        dao.list_activations_for(None, None)
            .await
            .expect("list")
            .len(),
        2,
        "\"the whole plugin, except this skill\" needs both records to survive"
    );

    assert!(
        dao.clear_activation("p1", Some("noisy".to_string()), None, None)
            .await
            .expect("clear the skill record")
    );
    assert_eq!(
        dao.list_activations_for(None, None)
            .await
            .expect("list")
            .len(),
        1,
        "clearing the skill record must not take the plugin record with it"
    );
}

/// A real workspace row: an activation record's `workspace_id` is a foreign
/// key, so a Workspace record cannot be written for one that does not exist.
async fn workspace(pool: &PgPool, id: &str) -> String {
    sqlx::query("INSERT INTO workspaces (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("workspace-{id}"))
        .execute(pool)
        .await
        .expect("insert workspace");
    id.to_string()
}

#[sqlx::test]
async fn a_workspace_record_sits_beside_the_global_and_personal_ones(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let alice_id = user(&pool, "alice").await;
    let ws = workspace(&pool, "ws1").await;

    for row in [
        write(None, None, None, true),
        write(None, Some(alice_id), None, false),
        write(None, None, Some(&ws), true),
    ] {
        dao.set_activation(&row).await.expect("set");
    }

    let rows = dao
        .list_activations_for(Some(alice_id), Some(&ws))
        .await
        .expect("list");
    assert_eq!(
        rows.len(),
        3,
        "a workspace record shares the (plugin, skill) key with the global one \
         and must not replace it"
    );
}

#[sqlx::test]
async fn one_workspace_does_not_see_another_workspaces_records(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let mine = workspace(&pool, "ws1").await;
    let theirs = workspace(&pool, "ws2").await;

    dao.set_activation(&write(None, None, Some(&theirs), false))
        .await
        .expect("set");

    assert!(
        dao.list_activations_for(None, Some(&mine))
            .await
            .expect("list")
            .is_empty(),
        "narrowing a plugin in one workspace must not narrow it in every other"
    );
}

#[sqlx::test]
async fn clearing_a_workspace_record_leaves_the_personal_one_alone(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let alice_id = user(&pool, "alice").await;
    let ws = workspace(&pool, "ws1").await;

    for row in [
        write(None, Some(alice_id), None, true),
        write(None, None, Some(&ws), false),
    ] {
        dao.set_activation(&row).await.expect("set");
    }

    assert!(
        dao.clear_activation("p1", None, None, Some(&ws))
            .await
            .expect("clear")
    );

    let rows = dao
        .list_activations_for(Some(alice_id), Some(&ws))
        .await
        .expect("list");
    assert_eq!(
        rows.iter().map(|r| r.user_id).collect::<Vec<_>>(),
        vec![Some(alice_id)],
        "the two records key apart, so clearing one must not take the other"
    );
}

#[sqlx::test]
async fn a_record_cannot_name_both_a_person_and_a_workspace(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let alice_id = user(&pool, "alice").await;
    let ws = workspace(&pool, "ws1").await;

    let both = dao
        .set_activation(&ActivationWrite {
            plugin_id: "p1".into(),
            user_id: Some(alice_id),
            workspace_id: Some(ws),
            enabled: true,
            ..Default::default()
        })
        .await;

    assert!(
        both.is_err(),
        "a row naming two subjects has no place in the precedence order, so \
         the database refuses it rather than leaving resolution to pick"
    );
}

#[sqlx::test]
async fn deleting_a_workspace_takes_its_activations_with_it(pool: PgPool) {
    let dao = PgPluginDao(pool.clone());
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    let ws = workspace(&pool, "ws1").await;

    dao.set_activation(&write(None, None, Some(&ws), true))
        .await
        .expect("set");

    sqlx::query("DELETE FROM workspaces WHERE id = $1")
        .bind(&ws)
        .execute(&pool)
        .await
        .expect("delete workspace");

    assert!(
        dao.list_activations_for(None, Some(&ws))
            .await
            .expect("list")
            .is_empty(),
        "records outliving the workspace they narrow would apply to whatever \
         reused the id"
    );
}

#[sqlx::test]
async fn deleting_a_plugin_takes_its_activations_with_it(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");
    dao.set_activation(&write(None, None, None, true))
        .await
        .expect("set");

    assert!(dao.delete("p1").await.expect("delete"));
    assert!(
        dao.list_activations_for(None, None)
            .await
            .expect("list")
            .is_empty(),
        "a removed plugin must stop being delivered, records and all"
    );
}

#[sqlx::test]
async fn a_write_that_says_nothing_about_a_flag_leaves_it_standing(pool: PgPool) {
    let dao = PgPluginDao(pool);
    dao.insert(&plugin("p1")).await.expect("insert plugin");

    let stored = dao
        .set_activation(&ActivationWrite {
            plugin_id: "p1".into(),
            enabled: true,
            always: Some(true),
            frontload: Some(true),
            ..Default::default()
        })
        .await
        .expect("set with both flags");
    assert!(
        stored.always && stored.frontload,
        "the record is read back rather than the request echoed"
    );

    let toggled = dao
        .set_activation(&write(None, None, None, false))
        .await
        .expect("turn off");
    assert!(
        toggled.always && toggled.frontload,
        "a control that knows only about on and off must not clear flags it \
         cannot see"
    );

    let cleared = dao
        .set_activation(&ActivationWrite {
            plugin_id: "p1".into(),
            enabled: true,
            always: Some(false),
            ..Default::default()
        })
        .await
        .expect("give up always");
    assert!(!cleared.always, "a flag named false is given up");
    assert!(
        cleared.frontload,
        "naming one flag must not take the other with it"
    );
}
