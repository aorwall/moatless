use chrono::Utc;
use cron::Schedule;

use crate::channel::error::ChannelError;

pub struct ScheduledJobService;

impl ScheduledJobService {
    pub fn next_fire_at(
        schedule_expr: &str,
    ) -> Result<chrono::DateTime<chrono::Utc>, ChannelError> {
        let schedule: Schedule = schedule_expr
            .parse()
            .map_err(|e| ChannelError::BadRequest(format!("Invalid schedule_expr: {e}")))?;
        schedule
            .after(&Utc::now())
            .next()
            .ok_or_else(|| ChannelError::BadRequest("schedule_expr has no future fire time".into()))
    }
}
