use moatless_workspace_types::workspace::Memory;

use super::*;

#[test]
fn slugify_produces_stable_keys() {
    assert_eq!(slugify("API Server"), "api-server");
    assert_eq!(slugify("  Web / Frontend  "), "web-frontend");
    assert_eq!(slugify("a__b--c"), "a-b-c");
    assert_eq!(slugify("!!!"), "workspace");
    assert_eq!(slugify(""), "workspace");
}

#[test]
fn repo_name_from_url_takes_last_segment() {
    assert_eq!(repo_name_from_url("https://github.com/acme/api.git"), "api");
    assert_eq!(repo_name_from_url("https://github.com/acme/web/"), "web");
    assert_eq!(repo_name_from_url("https://gitlab.com/g/sub/proj"), "proj");
}

/// One entry of a `.moatless/workspaces.json`, parsed.
fn parse_entry(json: &str) -> MoatlessWorkspaceEntry {
    serde_json::from_str(json).expect("parse workspace entry")
}

fn grant(json: &str) -> WorkspaceIdentityGrant {
    serde_json::from_str(json).expect("parse grant")
}

/// Repo config may not hand a sandbox an elevated Kubernetes identity —
/// otherwise anyone with commit access could grant themselves the access an
/// administrator withholds.
#[test]
fn git_declared_privileged_identity_without_a_grant_is_stripped() {
    let entry = parse_entry(
        r#"{
            "name": "API",
            "dockerImage": "myregistry/custom-sandbox:latest",
            "systemPrompt": "Be helpful",
            "sandboxServiceAccount": "moatless-admin",
            "networkProfile": "moatless-admin",
            "nodeSelector": { "workload": "moatless-od" }
        }"#,
    );

    let mut run = run_config_from_entry(&entry);
    let stripped = apply_identity_grant(&mut run, None, "repo-1", "api");

    assert_eq!(
        stripped,
        ["sandboxServiceAccount", "networkProfile", "nodeSelector"]
    );
    assert!(run.sandbox_service_account.is_none());
    assert!(run.network_profile.is_none());
    assert!(run.node_selector.is_none());
    // Everything else the repo declares still syncs.
    assert_eq!(
        run.docker_image,
        Some("myregistry/custom-sandbox:latest".to_string())
    );
    assert_eq!(run.system_prompt, Some("Be helpful".to_string()));
}

#[test]
fn a_granted_identity_is_kept_and_a_different_value_is_not() {
    let entry = parse_entry(
        r#"{
            "name": "Ops",
            "sandboxServiceAccount": "moatless-cluster-reader",
            "networkProfile": "soap-nonprod-admin",
            "nodeSelector": { "workload": "moatless-od" }
        }"#,
    );
    let grant = grant(
        r#"{
            "repository": "soaplabs/ops", "workspace": "ops",
            "sandboxServiceAccount": "moatless-cluster-reader",
            "networkProfile": "cluster-reader",
            "nodeSelector": { "workload": "moatless-od" }
        }"#,
    );

    let mut run = run_config_from_entry(&entry);
    let stripped = apply_identity_grant(&mut run, Some(&grant), "repo-1", "ops");

    assert_eq!(stripped, ["networkProfile"]);
    assert_eq!(
        run.sandbox_service_account.as_deref(),
        Some("moatless-cluster-reader")
    );
    assert!(run.network_profile.is_none());
    assert_eq!(
        run.node_selector
            .unwrap()
            .get("workload")
            .map(String::as_str),
        Some("moatless-od")
    );
}

/// A grant permits a value; it does not impose one on an entry declaring none.
#[test]
fn a_grant_sets_nothing_the_entry_does_not_declare() {
    let entry = parse_entry(r#"{ "name": "API" }"#);
    let grant = grant(
        r#"{ "repository": "a/b", "workspace": "api", "sandboxServiceAccount": "moatless-admin" }"#,
    );

    let mut run = run_config_from_entry(&entry);
    let stripped = apply_identity_grant(&mut run, Some(&grant), "repo-1", "api");

    assert!(stripped.is_empty());
    assert!(run.sandbox_service_account.is_none());
}

#[test]
fn a_default_entry_needs_neither_a_name_nor_repos() {
    let entry = parse_entry(r#"{ "key": "default", "dockerImage": "img" }"#);

    assert_eq!(
        source_key_for(&entry).as_deref(),
        Some(DEFAULT_WORKSPACE_KEY)
    );
    assert!(entry.name.is_none());
    assert!(entry.repos.is_empty());
    assert!(source_key_for(&parse_entry("{}")).is_none());
}

#[test]
fn config_carries_storage_switches_from_git_repo() {
    // A repo turning the PVC off and keeping the S3 snapshot. The nested
    // `ResourceConfig` must read the camelCase keys and round-trip back to
    // camelCase in the stored JSONB so the provision-time deserialize reads
    // it correctly.
    let entry = parse_entry(
        r#"{
            "name": "API",
            "repos": [],
            "resources": { "pvcEnabled": false, "s3Enabled": true, "storage": "8Gi" }
        }"#,
    );
    let resources = entry.resources.as_ref().expect("resources present");
    assert_eq!(resources.pvc_enabled, Some(false));
    assert_eq!(resources.s3_enabled, Some(true));
    assert_eq!(resources.storage.as_ref().map(Memory::as_str), Some("8Gi"));

    let run = run_config_from_entry(&entry);
    let stored = run.resources.expect("resources value present");
    let value = serde_json::to_value(&stored).expect("serialize");
    assert_eq!(value.get("pvcEnabled"), Some(&serde_json::json!(false)));
    assert_eq!(value.get("s3Enabled"), Some(&serde_json::json!(true)));
    assert_eq!(value.get("storage"), Some(&serde_json::json!("8Gi")));
    assert!(
        value.get("pvc_enabled").is_none(),
        "snake_case key must not leak into stored JSONB"
    );
}

/// The removed `storageMode` is an unknown field, not an error: a repo that
/// still carries one loads and simply falls back to the deployment
/// defaults.
#[test]
fn a_stale_storage_mode_is_ignored() {
    let entry = parse_entry(
        r#"{
            "name": "API",
            "repos": [],
            "resources": { "storageMode": "ephemeral", "storage": "8Gi" }
        }"#,
    );
    let resources = entry.resources.as_ref().expect("resources present");
    assert_eq!(resources.pvc_enabled, None);
    assert_eq!(resources.s3_enabled, None);
    assert_eq!(resources.storage.as_ref().map(Memory::as_str), Some("8Gi"));
}

#[test]
fn parse_workspaces_json() {
    let json = r#"{
        "version": 1,
        "workspaces": [
            {
                "key": "api",
                "name": "API",
                "description": "backend",
                "repos": [
                    { "url": "https://github.com/acme/api", "primary": true },
                    { "url": "https://github.com/acme/shared", "mountName": "shared" }
                ],
                "dockerImage": "acme/sandbox:latest",
                "codeExecution": { "setupCommands": ["bun install"] },
                "scripts": [{ "name": "dev", "command": "bun dev", "port": 3000 }],
                "envVars": { "LOG_LEVEL": "debug" },
                "networkProfile": "sealed"
            },
            { "name": "Web Only", "repos": [{ "url": "https://github.com/acme/web" }] }
        ]
    }"#;

    let file: MoatlessWorkspacesFile = serde_json::from_str(json).unwrap();
    assert_eq!(file.version, Some(1));
    assert_eq!(file.workspaces.len(), 2);

    let api = &file.workspaces[0];
    assert_eq!(api.key.as_deref(), Some("api"));
    assert_eq!(api.name.as_deref(), Some("API"));
    assert_eq!(api.repos.len(), 2);
    assert!(api.repos[0].primary);
    assert_eq!(api.repos[0].url, "https://github.com/acme/api");
    assert_eq!(api.repos[1].mount_name.as_deref(), Some("shared"));
    assert!(!api.repos[1].primary);
    assert_eq!(api.docker_image.as_deref(), Some("acme/sandbox:latest"));
    assert_eq!(
        api.code_execution
            .as_ref()
            .unwrap()
            .setup_commands
            .as_ref()
            .unwrap(),
        &vec!["bun install".to_string()]
    );
    assert_eq!(api.network_profile.as_deref(), Some("sealed"));

    // Missing key → resolved later from the name; primary defaults to false.
    let web = &file.workspaces[1];
    assert!(web.key.is_none());
    assert_eq!(web.name.as_deref(), Some("Web Only"));
    assert!(!web.repos[0].primary);
    assert!(unknown_key_warnings(&file).is_empty());
}

/// A known id applies and an unknown one is named rather than failing the
/// file, so a repo can list a server a newer backend adds.
#[test]
fn language_servers_keep_known_ids_and_name_unknown_ones() {
    let json = r#"{
        "workspaces": [{
            "name": "API",
            "repos": [],
            "languageServers": ["rust", "cobol", "typescript"]
        }]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

    assert_eq!(
        run_config_from_entry(&file.workspaces[0]).language_servers,
        Some(vec![LanguageServer::Rust, LanguageServer::Typescript])
    );
    assert!(unknown_key_warnings(&file).is_empty());
    let warnings = unknown_language_server_warnings(&file);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].path, "workspaces[0].languageServers[1]");
    assert_eq!(warnings[0].message, UNKNOWN_LANGUAGE_SERVER);
}

/// Linear team keys are upper case and an issue's identifier carries its
/// team's key, so a lowercase or padded key would match no issue.
#[test]
fn a_linear_team_is_read_as_its_upper_case_key() {
    let json = r#"{
        "workspaces": [
            { "name": "API", "repos": [], "linearTeam": " sp " },
            { "name": "Web", "repos": [], "linearTeam": "  " }
        ]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

    assert_eq!(
        run_config_from_entry(&file.workspaces[0])
            .linear_team
            .as_deref(),
        Some("SP")
    );
    assert!(
        run_config_from_entry(&file.workspaces[1])
            .linear_team
            .is_none()
    );
    assert!(unknown_key_warnings(&file).is_empty());
}

/// The Task defaults and the icon reach the row from git, so changing one
/// no longer costs an override; an absent key clears the column.
#[test]
fn task_defaults_and_icon_sync_from_git() {
    let json = r#"{
        "workspaces": [
            {
                "name": "API",
                "repos": [],
                "defaultModel": "opus",
                "defaultEffort": "high",
                "icon": { "kind": "lucide", "name": "rocket", "color": "blue" }
            },
            { "name": "Web", "repos": [] }
        ]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

    let api = run_config_from_entry(&file.workspaces[0]);
    assert_eq!(api.default_model.as_deref(), Some("opus"));
    assert_eq!(api.default_effort.as_deref(), Some("high"));
    assert_eq!(
        api.icon,
        Some(WorkspaceIcon::Lucide {
            name: "rocket".to_string(),
            color: "blue".to_string(),
        })
    );
    let web = run_config_from_entry(&file.workspaces[1]);
    assert!(web.default_model.is_none());
    assert!(web.default_effort.is_none());
    assert!(web.icon.is_none());
    assert!(unknown_key_warnings(&file).is_empty());
    assert!(invalid_icon_warnings(&file).is_empty());
}

/// A malformed icon is named and dropped rather than failing the file:
/// a glyph must not stop every other Workspace in it syncing.
#[test]
fn an_icon_of_unknown_shape_is_named_rather_than_failing_the_file() {
    let json = r#"{
        "workspaces": [{
            "name": "API",
            "repos": [],
            "icon": { "kind": "svg", "path": "logo.svg" }
        }]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

    assert!(run_config_from_entry(&file.workspaces[0]).icon.is_none());
    let warnings = invalid_icon_warnings(&file);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].path, "workspaces[0].icon");
    assert_eq!(warnings[0].message, INVALID_ICON);
}

/// `skills` is a recognised key, a stray one inside it is named, and an
/// entry without it declares nothing, which leaves its records alone.
#[test]
fn skills_resolve_against_the_registry_and_stray_keys_inside_are_named() {
    let json = r#"{
        "workspaces": [
            { "name": "API", "repos": [], "skills": { "enable": ["general"], "only": [] } },
            { "name": "Web", "repos": [] }
        ]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");
    let registry = (
        vec![Plugin {
            id: "p1".to_string(),
            name: "general".to_string(),
            git_url: "https://github.com/acme/general".to_string(),
            source: moatless_workspace_types::plugin::PluginSource::Manual,
            synced_version: None,
            synced_at: None,
            disabled_at: None,
            created_at: None,
            updated_at: None,
        }],
        Vec::new(),
    );

    let paths: Vec<String> = unknown_key_warnings(&file)
        .into_iter()
        .map(|w| w.path)
        .collect();
    assert_eq!(paths, ["workspaces[0].skills.only"]);

    let mut warnings = Vec::new();
    let api = declared_activations(&file.workspaces[0], Some(&registry), 0, &mut warnings);
    assert_eq!(
        api,
        Some(vec![DeclaredActivation {
            plugin_id: "p1".to_string(),
            skill_name: None,
            enabled: true,
        }])
    );
    assert!(warnings.is_empty());
    assert_eq!(
        declared_activations(&file.workspaces[1], Some(&registry), 1, &mut warnings),
        None
    );
}

/// The shape that cost two agents a session: `previewConfig` and
/// `codeExecution.envVars` parse without error, nothing reads them, and the
/// committed file reads as authoritative. Each one has to be named.
#[test]
fn unknown_keys_are_named_with_the_path_they_sit_at() {
    let json = r#"{
        "version": 1,
        "previewConfig": { "serverStartCommands": ["bun run dev --turbo"] },
        "workspaces": [
            {
                "name": "Frontend",
                "repos": [{ "url": "https://github.com/acme/web", "revision": "main" }],
                "previewConfig": { "serverStartCommands": ["bun run dev"] },
                "codeExecution": { "envVars": { "VITE_BACKEND_URL": "https://api" } }
            }
        ]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

    let warnings = unknown_key_warnings(&file);
    let paths: Vec<String> = warnings
        .iter()
        .map(|warning| warning.path.clone())
        .collect();
    assert_eq!(
        paths,
        [
            "previewConfig",
            "workspaces[0].previewConfig",
            "workspaces[0].codeExecution.envVars",
            "workspaces[0].repos[0].revision",
        ]
    );
    assert!(warnings.iter().all(|w| w.message == UNKNOWN_KEY));
}

/// A key the schema does claim must never be reported: a warning on a
/// working config is the same misdirection as silence on a broken one. A
/// `"//"` comment is not a mistake either — `workspaces.shadow.json` is
/// written that way.
#[test]
fn a_recognised_key_or_a_comment_is_not_reported_as_unknown() {
    let json = r#"{
        "//": "the shadow workspace",
        "workspaces": [{
            "//1": "runs two scripts",
            "name": "Frontend",
            "repos": [{ "url": "https://github.com/acme/web", "mountName": "web" }],
            "codeExecution": { "setupCommands": ["bun install"] },
            "envVars": { "LOG_LEVEL": "debug" }
        }]
    }"#;
    let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

    assert!(unknown_key_warnings(&file).is_empty());
}

#[test]
fn place_repository_false_places_nothing_only_on_a_non_default_entry_without_repos() {
    let blank = parse_entry(r#"{ "name": "Scratch", "placeRepository": false }"#);
    assert!(places_no_repository(&blank, false));
    // The repository's own Workspace always places it.
    assert!(!places_no_repository(&blank, true));

    let listed = parse_entry(
        r#"{ "name": "Pair", "placeRepository": false,
             "repos": [{ "url": "https://github.com/acme/web.git" }] }"#,
    );
    assert!(!places_no_repository(&listed, false));

    let unset = parse_entry(r#"{ "name": "API" }"#);
    assert!(!places_no_repository(&unset, false));
}
