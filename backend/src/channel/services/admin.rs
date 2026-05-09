use crate::channel::dao::bindings::PgTaskBindingRepository;
use crate::channel::dao::connections::PgProviderConnectionRepository;
use crate::channel::dao::routes::PgTaskRouteRepository;
use crate::channel::dao::scheduled_jobs::PgScheduledJobRepository;
use crate::channel::dao::trigger_routes::PgTaskTriggerRouteRepository;
use crate::channel::dao::triggers::PgTaskTriggerRepository;
use crate::db::DbPool;

pub struct IntegrationAdminService {
    pub connections: PgProviderConnectionRepository,
    pub routes: PgTaskRouteRepository,
    pub triggers: PgTaskTriggerRepository,
    pub trigger_routes: PgTaskTriggerRouteRepository,
    pub bindings: PgTaskBindingRepository,
    pub scheduled_jobs: PgScheduledJobRepository,
}

impl IntegrationAdminService {
    pub fn new(db: DbPool) -> Self {
        Self {
            connections: PgProviderConnectionRepository(db.clone()),
            routes: PgTaskRouteRepository(db.clone()),
            triggers: PgTaskTriggerRepository(db.clone()),
            trigger_routes: PgTaskTriggerRouteRepository(db.clone()),
            bindings: PgTaskBindingRepository(db.clone()),
            scheduled_jobs: PgScheduledJobRepository(db),
        }
    }
}
