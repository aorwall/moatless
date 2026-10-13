//! The environment a sandbox container is handed: image resolution, repo
//! projection, credential redaction and the toolchain flags.

use super::fixtures::*;
use super::*;

#[test]
fn resolve_sandbox_image_prefers_repo_override() {
    let config = test_config();

    let plan = test_plan();
    assert_eq!(resolve_sandbox_image(&config, &plan), "sandbox:latest");

    // case: empty override → treated as unset → config default
    let mut plan = test_plan();
    plan.repo_config.docker_image = Some(String::new());
    assert_eq!(resolve_sandbox_image(&config, &plan), "sandbox:latest");

    let mut plan = test_plan();
    plan.repo_config.docker_image = Some("custom:v2".into());
    assert_eq!(resolve_sandbox_image(&config, &plan), "custom:v2");
}

#[test]
fn chrono_now_iso_is_valid_rfc3339() {
    let ts = chrono_now_iso();
    // The created-at annotation must be valid RFC3339.
    assert!(
        chrono::DateTime::parse_from_rfc3339(&ts).is_ok(),
        "created-at annotation must be valid RFC3339, got {ts:?}"
    );
    assert!(ts.ends_with('Z'), "expected UTC 'Z' suffix, got {ts:?}");
}

#[test]
fn redact_url_userinfo_strips_credentials() {
    assert_eq!(
        redact_url_userinfo("https://user:pat@github.com/owner/repo.git"),
        "https://github.com/owner/repo.git"
    );
    assert_eq!(
        redact_url_userinfo("https://oauth2:glpat-xxx@gitlab.com/group/repo.git"),
        "https://gitlab.com/group/repo.git"
    );
    assert_eq!(
        redact_url_userinfo("https://github.com/owner/repo.git"),
        "https://github.com/owner/repo.git"
    );
    // A '@' in the path (not the authority) must not be treated as userinfo.
    assert_eq!(
        redact_url_userinfo("https://host/owner/repo@v1.git"),
        "https://host/owner/repo@v1.git"
    );
    // Non-HTTP(S) (scp-like SSH) forms are left untouched.
    assert_eq!(
        redact_url_userinfo("git@github.com:owner/repo.git"),
        "git@github.com:owner/repo.git"
    );
}

#[test]
fn pod_spec_redacts_clone_url_and_omits_user_token() {
    let config = test_config();
    let mut plan = test_plan();
    plan.git_clone_url = "https://user:secret@github.com/test/repo.git".into();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();

    // Annotation must not carry the embedded credential.
    let anno = pod
        .metadata
        .annotations
        .as_ref()
        .unwrap()
        .get("moatless.io/repository-url")
        .unwrap();
    assert_eq!(anno, "https://github.com/test/repo.git");
    assert!(!anno.contains("secret"), "annotation leaked credentials");

    // GIT_CLONE_URL env must be credential-free too.
    let sandbox = pod
        .spec
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap();
    let env = sandbox.env.as_ref().unwrap();
    let git_url = env.iter().find(|e| e.name == "GIT_CLONE_URL").unwrap();
    assert_eq!(
        git_url.value.as_deref(),
        Some("https://github.com/test/repo.git")
    );

    assert!(env.iter().all(|e| e.name != "USER_AUTH_TOKEN"));
    assert!(env.iter().all(|e| e.name != "SANDBOX_BOOTSTRAP_TOKEN"));
}

/// A multi-repo workspace reaches the pod as the `WORKSPACE_REPOS` list, while
/// `GIT_CLONE_URL` still names the primary so a sandbox image predating
/// multi-repo support keeps working.
#[test]
fn pod_spec_carries_every_workspace_repo() {
    let config = test_config();
    let mut plan = test_plan();
    plan.repos.push(RepoCheckout {
        uses_ssh: false,
        repository_id: "repo-2".into(),
        clone_url: "https://gitlab.com/acme/shared.git".into(),
        branch: Some("release".into()),
        provider: "gitlab".into(),
        mount_name: "shared".into(),
        is_primary: false,
    });

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let sandbox = pod
        .spec
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap();
    let env = sandbox.env.as_ref().unwrap();
    let raw = env
        .iter()
        .find(|e| e.name == WORKSPACE_REPOS_ENV)
        .and_then(|e| e.value.clone())
        .expect("WORKSPACE_REPOS present");
    let parsed: Vec<RepoCheckout> = serde_json::from_str(&raw).unwrap();

    assert_eq!(parsed.len(), 2);
    assert!(parsed[0].is_primary);
    assert_eq!(parsed[0].mount_name, "repo");
    assert_eq!(parsed[1].mount_name, "shared");
    assert_eq!(parsed[1].provider, "gitlab");
    assert_eq!(parsed[1].branch.as_deref(), Some("release"));

    let git_url = env.iter().find(|e| e.name == "GIT_CLONE_URL").unwrap();
    assert_eq!(
        git_url.value.as_deref(),
        Some("https://github.com/test/repo.git")
    );
}

/// Every checkout's URL is redacted, not just the primary's — a secondary
/// repo's PAT must not land in the pod spec either.
#[test]
fn workspace_repos_env_redacts_every_clone_url() {
    let config = test_config();
    let mut plan = test_plan();
    plan.repos[0].clone_url = "https://user:secret@github.com/test/repo.git".into();
    plan.repos.push(RepoCheckout {
        uses_ssh: false,
        repository_id: "repo-2".into(),
        clone_url: "https://user:other-secret@gitlab.com/acme/shared.git".into(),
        branch: None,
        provider: "gitlab".into(),
        mount_name: "shared".into(),
        is_primary: false,
    });

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let raw = pod
        .spec
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap()
        .env
        .as_ref()
        .unwrap()
        .iter()
        .find(|e| e.name == WORKSPACE_REPOS_ENV)
        .and_then(|e| e.value.clone())
        .expect("WORKSPACE_REPOS present");

    assert!(
        !raw.contains("secret"),
        "WORKSPACE_REPOS leaked credentials"
    );
    let parsed: Vec<RepoCheckout> = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed[0].clone_url, "https://github.com/test/repo.git");
    assert_eq!(parsed[1].clone_url, "https://gitlab.com/acme/shared.git");
}

/// A plan with no checkouts omits the env entirely rather than emitting an
/// empty list.
#[test]
fn pod_spec_omits_workspace_repos_when_the_plan_has_none() {
    let config = test_config();
    let mut plan = test_plan();
    plan.repos.clear();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let env = pod
        .spec
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap()
        .env
        .clone()
        .unwrap();
    assert!(env.iter().all(|e| e.name != WORKSPACE_REPOS_ENV));
}

#[test]
fn pod_spec_emits_bootstrap_and_optional_legacy_token() {
    let config = test_config();
    let plan = test_plan();

    // Bootstrap only.
    let pod = build_pod(
        &config,
        "task-1",
        &plan,
        None,
        Some(SandboxAuthTokens {
            bootstrap: "boot-tok",
            legacy_user_token: None,
        }),
    )
    .unwrap();
    let env = sandbox_env(&pod);
    assert_eq!(
        env.iter()
            .find(|e| e.name == "SANDBOX_BOOTSTRAP_TOKEN")
            .and_then(|e| e.value.as_deref()),
        Some("boot-tok")
    );
    assert!(
        env.iter().all(|e| e.name != "USER_AUTH_TOKEN"),
        "legacy USER_AUTH_TOKEN must be absent when not requested"
    );

    // Bootstrap plus USER_AUTH_TOKEN.
    let pod = build_pod(
        &config,
        "task-1",
        &plan,
        None,
        Some(SandboxAuthTokens {
            bootstrap: "boot-tok",
            legacy_user_token: Some("legacy-tok"),
        }),
    )
    .unwrap();
    let env = sandbox_env(&pod);
    assert_eq!(
        env.iter()
            .find(|e| e.name == "USER_AUTH_TOKEN")
            .and_then(|e| e.value.as_deref()),
        Some("legacy-tok")
    );
}

/// The Task's fixed Agent-harness selection reaches the sandbox container as
/// `AGENT_PROFILE` at construction, so the sandbox resolves its harness once
/// instead of switching per Message.
///
/// `auto_provision` stamps this into `plan.env_vars`; here we prove the builder
/// delivers it verbatim.
#[test]
fn agent_profile_env_reaches_the_sandbox_container() {
    let config = test_config();
    let mut plan = test_plan();
    plan.env_vars = Some(std::collections::HashMap::from([(
        "AGENT_PROFILE".to_string(),
        "codex".to_string(),
    )]));
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        sandbox_env(&pod)
            .iter()
            .find(|e| e.name == "AGENT_PROFILE")
            .and_then(|e| e.value.as_deref()),
        Some("codex"),
    );
}

/// The toolchain-persistence flag is off by default — no cargo subPath mounts.
#[test]
fn toolchain_persistence_flags_default_off() {
    let config = test_config();
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let mounts = container.volume_mounts.as_ref().unwrap();
    assert!(
        !mounts
            .iter()
            .any(|m| m.mount_path.starts_with("/opt/cargo")),
        "no cargo mounts without the persist flag"
    );
}

/// With the flag on: cargo registry/git ride `workspace-data` subPaths — never
/// all of /opt/cargo, which would shadow the baked toolchain.
#[test]
fn toolchain_persistence_flags_enabled() {
    let mut config = test_config();
    config.sandbox.persist_toolchain = true;
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let mounts = container.volume_mounts.as_ref().unwrap();
    for (mount_path, sub_path) in [
        ("/opt/cargo/registry", "cargo-registry"),
        ("/opt/cargo/git", "cargo-git"),
    ] {
        let m = mounts
            .iter()
            .find(|m| m.mount_path == mount_path)
            .unwrap_or_else(|| panic!("{mount_path} mount present"));
        assert_eq!(m.name, "workspace-data");
        assert_eq!(m.sub_path.as_deref(), Some(sub_path));
    }
    assert!(
        !mounts.iter().any(|m| m.mount_path == "/opt/cargo"),
        "must never shadow the baked toolchain at /opt/cargo"
    );
}

/// The volume limit is the one ceiling the sandbox cannot discover for itself:
/// the kubelet meters it, so `df` in the pod answers for the node instead.
#[test]
fn the_workspace_volume_limit_reaches_the_container_env() {
    let config = test_config();

    let mut plan = test_plan();
    plan.repo_config.resources = Some(moatless_workspace_types::workspace::ResourceConfig {
        storage: Some(memory("20Gi")),
        ..Default::default()
    });
    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let limit = container
        .env
        .as_ref()
        .unwrap()
        .iter()
        .find(|e| e.name == WORKSPACE_LIMIT_ENV)
        .and_then(|e| e.value.clone())
        .expect("SANDBOX_WORKSPACE_LIMIT present");
    assert_eq!(limit, "20Gi");

    // No per-repo override: the same knob that sizes the volume is exported.
    let container = build_sandbox_container(&config, "task-1", &test_plan(), None, None).unwrap();
    let limit = container
        .env
        .as_ref()
        .unwrap()
        .iter()
        .find(|e| e.name == WORKSPACE_LIMIT_ENV)
        .and_then(|e| e.value.clone())
        .expect("SANDBOX_WORKSPACE_LIMIT present");
    assert_eq!(limit, config.sandbox.storage_size);
}
