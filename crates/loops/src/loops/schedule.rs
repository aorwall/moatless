//! Cron schedule validation and the next due instant.

use cron::Schedule;

/// A cron expression is invalid or has no remaining occurrence.
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("Invalid cron expression: {0}")]
    Invalid(#[source] cron::error::Error),
    #[error("Schedule has no future fire time")]
    NoFutureTime,
}

pub(crate) fn next_fire_at(
    expression: &str,
) -> Result<chrono::DateTime<chrono::Utc>, ScheduleError> {
    let schedule: Schedule = expression.parse().map_err(ScheduleError::Invalid)?;
    schedule
        .after(&chrono::Utc::now())
        .next()
        .ok_or(ScheduleError::NoFutureTime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_cron_keeps_its_cause_and_client_message() {
        let error = next_fire_at("not a cron").unwrap_err();
        assert!(matches!(error, ScheduleError::Invalid(_)));
        assert!(error.to_string().starts_with("Invalid cron expression: "));
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn expired_schedule_is_distinct_from_invalid_cron() {
        let error = next_fire_at("0 0 0 1 1 * 2000").unwrap_err();
        assert!(matches!(error, ScheduleError::NoFutureTime));
        assert_eq!(error.to_string(), "Schedule has no future fire time");
    }
}
