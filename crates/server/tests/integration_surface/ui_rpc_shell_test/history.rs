//! A long thread read the way a V2 client opens it: the newest window over
//! `/bounded`, then older pages over `/history`, under the same access rule as
//! the whole snapshot.

use super::harness::{mounted_router, ui_rpc_authed_request};
use super::seeding::{seed_task, seed_turn};
use crate::common::USER_1;
use crate::common::USER_2;
use crate::common::body_json;
use crate::common::make_message_row;
use crate::common::make_workspace_row;
use axum::http::StatusCode;
use moatless_tasks::activities::mapping::ConversationWrite;
use moatless_workspaces::compose;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const TURNS: i32 = 30;

/// A thread of [`TURNS`] user turns, one message each, in `ws-history`.
async fn seed_long_thread(pool: &PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-history", USER_1))
        .await
        .unwrap();
    seed_task(pool, "t-long", USER_1, Some("ws-history")).await;
    for turn in 1..=TURNS {
        seed_turn(pool, "t-long", turn, "completed").await;
        let mut message = make_message_row("t-long", &format!("ask-{turn:02}"));
        message.turn_number = Some(turn);
        message.message_text = Some(format!("ask-{turn:02}"));
        moatless_tasks::message::test_support::seed_message(
            pool,
            &message,
            &[],
            &ConversationWrite::default(),
        )
        .await
        .unwrap();
    }
}

async fn get(router: &axum::Router, uri: &str, user_id: &str) -> (StatusCode, Value) {
    let mut request = ui_rpc_authed_request("GET", uri, user_id);
    request
        .headers_mut()
        .insert("x-t3-orchestration-protocol", "2".parse().unwrap());
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    (status, body_json(response).await)
}

/// What each row's message says, read back from `messages` where a deduped
/// body left the row blank.
fn message_texts(rows: &Value, messages: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .filter(|row| row["item"]["messageId"].is_string())
        .map(|row| {
            let item = &row["item"];
            let text = match item["text"].as_str() {
                Some("") | None => messages
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|message| message["id"] == item["messageId"])
                    .and_then(|message| message["text"].as_str()),
                text => text,
            };
            text.unwrap().to_string()
        })
        .collect()
}

fn cursor_query(cursor: &Value) -> String {
    cursor.as_str().unwrap().to_string()
}

#[sqlx::test]
async fn a_long_thread_opens_on_its_newest_turns_and_pages_the_rest(pool: PgPool) {
    seed_long_thread(&pool).await;
    let router = mounted_router(&pool).await;

    let (status, bounded) = get(
        &router,
        "/api/orchestration/threads/t-long/bounded?compactTurnItems=1&dedupeBodies=1",
        USER_1,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bounded}");
    assert_eq!(bounded["hasMoreHistory"], true);
    assert_eq!(bounded["turnItemsOmitLocalVisible"], true);
    assert!(bounded.get("ids").is_none(), "compactIds was not asked for");
    let projection = &bounded["projection"];
    let window = message_texts(&projection["visibleTurnItems"], &projection["messages"]);
    assert_eq!(window.len(), 10);
    assert_eq!(window[0], "ask-21");

    let (status, page) = get(
        &router,
        &format!(
            "/api/orchestration/threads/t-long/history?cursor={}",
            cursor_query(&bounded["historyCursor"])
        ),
        USER_1,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let older = message_texts(&page["items"], &Value::Array(Vec::new()));
    assert_eq!(older.len(), 20);
    assert_eq!(older[0], "ask-01");
    assert_eq!(older[19], "ask-20");
    assert_eq!(page["hasMoreHistory"], false);
    assert_eq!(page["nextCursor"], Value::Null);
}

#[sqlx::test]
async fn a_compacted_history_page_carries_an_id_table(pool: PgPool) {
    seed_long_thread(&pool).await;
    let router = mounted_router(&pool).await;
    let (_, bounded) = get(
        &router,
        "/api/orchestration/threads/t-long/bounded?compactIds=1",
        USER_1,
    )
    .await;
    assert!(bounded["ids"].is_array());
    assert!(bounded["projection"]["thread"]["id"].is_u64());

    let (_, page) = get(
        &router,
        &format!(
            "/api/orchestration/threads/t-long/history?compactIds=1&cursor={}",
            cursor_query(&bounded["historyCursor"])
        ),
        USER_1,
    )
    .await;

    let ids = page["ids"].as_array().unwrap();
    let first = &page["items"][0];
    assert_eq!(first["item"]["text"], "ask-01");
    let thread_id = &ids[first["sourceThreadId"].as_u64().unwrap() as usize];
    assert_eq!(thread_id, "t-long");
}

#[sqlx::test]
async fn a_thread_the_viewer_may_not_read_is_absent_from_both_routes(pool: PgPool) {
    seed_long_thread(&pool).await;
    let router = mounted_router(&pool).await;
    let (_, bounded) = get(&router, "/api/orchestration/threads/t-long/bounded", USER_1).await;
    let history = format!(
        "/api/orchestration/threads/t-long/history?cursor={}",
        cursor_query(&bounded["historyCursor"])
    );

    let (status, _) = get(&router, "/api/orchestration/threads/t-long/bounded", USER_2).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&router, &history, USER_2).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let forged = "/api/orchestration/threads/t-long/history?cursor=bm90LWEtY3Vyc29y";
    let (status, _) = get(&router, forged, USER_2).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "absence before the cursor");
}

#[sqlx::test]
async fn a_cursor_the_server_did_not_mint_is_a_typed_refusal(pool: PgPool) {
    seed_long_thread(&pool).await;
    let router = mounted_router(&pool).await;

    let (status, body) = get(
        &router,
        "/api/orchestration/threads/t-long/history?cursor=bm90LWEtY3Vyc29y",
        USER_1,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["_tag"], "EnvironmentRequestInvalidError");
    assert_eq!(body["reason"], "invalid_history_cursor");
}
