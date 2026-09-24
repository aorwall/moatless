//! Sandbox HTTP contracts re-exported from their canonical owner.

// The sandbox status-read response and its parts are request/response bodies, so
// they live in `moatless-api-schemas` (their single home); re-exported here so
// this crate's `schemas::` paths keep naming them.
pub use moatless_api_schemas::sandbox::{
    BackgroundWatchJobInfo, CommandSummary, ContainerDiagnostics, ManagedCommand,
    ManagedCommandState, RuntimeEvent, SandboxStatusFullResponse, ScheduledJobInfo,
    ScheduledJobKind, WakeupJobInfo,
};

pub use crate::file_conversion::UnknownFileNodeType;
pub use moatless_api_schemas::sandbox::{
    ContentOrigin, ContentSource, FileContentResponse, FileNameMatchDto, FileTreeNodeDto,
    FileTreeResponse, GitSummaryDto, ListFilesResponse, MoatlessToolName, ProvisionResponse,
    SearchByNameRequest, SearchByNameResponse, WriteFileRequestBody,
};
#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_status::{AgentStatus, SandboxStatus};

    #[test]
    fn provision_response_serializes_camel_case() {
        let resp = ProvisionResponse {
            status: SandboxStatus::Initializing,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "initializing");
    }

    #[test]
    fn error_body_serializes() {
        let body = moatless_api_schemas::error::ErrorBody {
            error: "not found".into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["error"], "not found");
    }

    #[test]
    fn scheduled_job_serializes_with_tagged_kind() {
        let wakeup = ScheduledJobInfo {
            id: "j2".into(),
            registered_at_unix_ms: 2,
            kind: ScheduledJobKind::Wakeup(WakeupJobInfo {
                reason: "ping".into(),
                prompt_preview: "wake up".into(),
                wakeup_at_unix_ms: 5,
            }),
        };
        let json = serde_json::to_value(&wakeup).unwrap();
        assert_eq!(json["id"], "j2");
        assert_eq!(json["registeredAtUnixMs"], 2);
        assert_eq!(json["kind"], "wakeup");
        assert_eq!(json["reason"], "ping");
        assert_eq!(json["promptPreview"], "wake up");
        assert_eq!(json["wakeupAtUnixMs"], 5);
    }

    #[test]
    fn empty_scheduled_jobs_omitted_from_response() {
        let resp = SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: None,
            agent_status: Some(AgentStatus::Idle),
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: vec![],
            commands: vec![],
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("scheduledJobs").is_none());
        assert!(json.get("commands").is_none());
    }

    #[test]
    fn populated_scheduled_jobs_serialized_in_response() {
        let resp = SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: None,
            agent_status: Some(AgentStatus::Running),
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: vec![ScheduledJobInfo {
                id: "j1".into(),
                registered_at_unix_ms: 1,
                kind: ScheduledJobKind::Wakeup(WakeupJobInfo {
                    reason: "ping".into(),
                    prompt_preview: "wake up".into(),
                    wakeup_at_unix_ms: 5,
                }),
            }],
            commands: vec![],
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let arr = json
            .get("scheduledJobs")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], "wakeup");
        assert_eq!(arr[0]["reason"], "ping");
    }

    #[test]
    fn commands_serialized_camel_case() {
        let resp = SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: None,
            agent_status: Some(AgentStatus::Idle),
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: vec![],
            commands: vec![ManagedCommand {
                id: "cmd-1".into(),
                command: "cargo test --no-run".into(),
                label: "cargo test --no-run".into(),
                pid: 1234,
                started_at_unix_ms: 1,
                timeout_secs: 3600,
                deadline_unix_ms: 2,
                state: ManagedCommandState::Running,
                exit_code: None,
                log_path: "/tmp/moat-commands/t/cmd-1.log".into(),
                finished_at_unix_ms: 0,
            }],
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let arr = json
            .get("commands")
            .and_then(|v| v.as_array())
            .expect("commands present when non-empty");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["pid"], 1234);
        assert_eq!(arr[0]["command"], "cargo test --no-run");
        assert_eq!(arr[0]["state"], "running");
        assert_eq!(arr[0]["startedAtUnixMs"], 1);
    }

    #[test]
    fn background_watch_scheduled_job_serialized_with_camel_case_kind() {
        let job = ScheduledJobInfo {
            id: "bg1".into(),
            registered_at_unix_ms: 7,
            kind: ScheduledJobKind::BackgroundWatch(BackgroundWatchJobInfo {
                command_count: 3,
                commands: vec!["sleep 30".into()],
            }),
        };
        let json = serde_json::to_value(&job).unwrap();
        assert_eq!(json["kind"], "backgroundWatch");
        assert_eq!(json["commandCount"], 3);
        assert_eq!(json["commands"][0], "sleep 30");
    }
}
