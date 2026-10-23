//! The two Loop declarations only a GitHub check-run trigger loop may make.
//!
//! A `<owner>/*` source matcher and `workspace: auto` both mean "any
//! repository of the org", which is safe only when a trusted App's check run,
//! not a pull request opening, creates the Task
//! (`adapter_config.create_on: check_run`, read by the GitHub Adapter).

/// `workspace:` value asking for each Task's Workspace to be resolved from the
/// pull request's repository.
pub(crate) const AUTO_WORKSPACE: &str = "auto";

/// Whether `source_matcher` is an org wildcard, `<owner>/*`.
pub(crate) fn is_org_wildcard(source_matcher: &str) -> bool {
    source_matcher
        .strip_suffix("/*")
        .is_some_and(|owner| !owner.is_empty() && !owner.contains('/'))
}

/// Whether `adapter_config` makes a loop create Tasks on trigger check runs only.
pub(crate) fn creates_on_check_run(adapter_config: Option<&serde_json::Value>) -> bool {
    adapter_config
        .and_then(|config| config.get("create_on"))
        .and_then(serde_json::Value::as_str)
        == Some("check_run")
}

type ListEntryCheck = fn(&serde_json::Value) -> bool;

/// Why a GitHub loop's source is refused, if it is: an org wildcard on a loop
/// that would create a Task for every pull request opened in the org, or a
/// `create_on: check_run` loop whose `check_run.names` or `app_ids` is empty,
/// which the adapter would never fire.
pub(crate) fn adapter_source_error(
    adapter_kind: &str,
    source_matcher: &str,
    adapter_config: Option<&serde_json::Value>,
) -> Option<String> {
    if adapter_kind != "github" {
        return None;
    }
    if !creates_on_check_run(adapter_config) {
        return is_org_wildcard(source_matcher).then(|| {
            format!("source matcher `{source_matcher}` needs `adapterConfig.create_on: check_run`")
        });
    }
    let check_run = adapter_config.and_then(|config| config.get("check_run"));
    // Mirrors what the adapter keeps (`as_str` names, `as_u64` ids): a quoted id
    // would otherwise sync and never fire.
    let checks: [(&str, &str, ListEntryCheck); 2] = [
        ("names", "check run names", |value| {
            value.as_str().is_some_and(|s| !s.is_empty())
        }),
        ("app_ids", "numeric App ids", |value| {
            value.as_u64().is_some()
        }),
    ];
    checks.into_iter().find_map(|(key, what, valid)| {
        let listed = check_run
            .and_then(|check_run| check_run.get(key))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|values| !values.is_empty() && values.iter().all(valid));
        (!listed).then(|| {
            format!("`adapterConfig.create_on: check_run` needs `check_run.{key}` to list {what}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_org_wildcard_needs_create_on_check_run() {
        let review = serde_json::json!({
            "create_on": "check_run",
            "check_run": {"names": ["soap-review-request"], "app_ids": [5_421_124]}
        });
        assert_eq!(
            adapter_source_error("github", "soaplabs/*", Some(&review)),
            None
        );
        assert!(adapter_source_error("github", "soaplabs/*", None).is_some());
        assert!(
            adapter_source_error(
                "github",
                "soaplabs/*",
                Some(&serde_json::json!({"create_on": "pr_opened"}))
            )
            .is_some()
        );
    }

    #[test]
    fn a_check_run_trigger_needs_names_and_app_ids() {
        let config = |check_run: serde_json::Value| serde_json::json!({"create_on": "check_run", "check_run": check_run});
        let error = |config: serde_json::Value| {
            adapter_source_error("github", "soaplabs/repo", Some(&config))
        };
        assert!(
            error(serde_json::json!({"create_on": "check_run"}))
                .is_some_and(|e| e.contains("check_run.names"))
        );
        assert!(
            error(config(serde_json::json!({"names": ["x"]})))
                .is_some_and(|e| e.contains("check_run.app_ids"))
        );
        assert!(error(config(serde_json::json!({"names": [], "app_ids": [1]}))).is_some());
        assert!(
            error(config(
                serde_json::json!({"names": ["x"], "app_ids": ["5421124"]})
            ))
            .is_some()
        );
        assert!(error(config(serde_json::json!({"names": [7], "app_ids": [1]}))).is_some());
        assert_eq!(
            error(config(serde_json::json!({"names": ["x"], "app_ids": [1]}))),
            None
        );
    }

    #[test]
    fn other_matchers_are_left_alone() {
        assert_eq!(adapter_source_error("github", "soaplabs/repo", None), None);
        assert_eq!(adapter_source_error("webhook", "*", None), None);
        assert_eq!(adapter_source_error("linear", "team/*", None), None);
        assert!(!is_org_wildcard("a/b/*"));
        assert!(!is_org_wildcard("/*"));
    }
}
