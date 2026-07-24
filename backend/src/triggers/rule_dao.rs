//! Canonical trigger+route view.
//!
//! The SQL was absorbed into [`crate::triggers::store`] (`LoopStore`). This
//! module keeps the `PgTaskTriggerRouteRepository` newtype as a thin facade
//! delegating to `LoopStore`, so existing call sites (including the out-of-scope
//! `github_pr` / `linear` runtimes) keep compiling unchanged. A later ticket
//! migrates them onto `AppState::loops` directly.

use crate::db::DbPool;
use crate::triggers::store::LoopStore;

use crate::adapters::models::TaskTriggerRoute;

pub struct PgTaskTriggerRouteRepository(pub DbPool);

impl PgTaskTriggerRouteRepository {
    fn store(&self) -> LoopStore {
        LoopStore::new(self.0.clone())
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        self.store().find_route_by_id(id).await
    }

    pub async fn find_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        self.store()
            .find_route_by_source_matcher(provider, source_matcher)
            .await
    }

    pub async fn list_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        self.store()
            .list_routes_by_source_matcher(provider, source_matcher)
            .await
    }

    pub async fn find_wildcard(
        &self,
        provider: &str,
    ) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        self.store().find_route_wildcard(provider).await
    }

    pub async fn find_by_connection_id(
        &self,
        connection_id: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        self.store().find_routes_by_connection_id(connection_id).await
    }

    pub async fn list_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        self.store().list_routes_by_provider(provider).await
    }

    pub async fn upsert(&self, row: &TaskTriggerRoute) -> Result<TaskTriggerRoute, sqlx::Error> {
        self.store().upsert_route(row).await
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        self.store().delete(id).await
    }
}
