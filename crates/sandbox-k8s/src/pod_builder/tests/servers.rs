//! Which declared server ports a pod admits, and what each server container
//! carries.

use super::fixtures::*;
use super::*;

#[test]
fn admitted_server_ports_drops_reserved_infra_ports() {
    let config = test_config();
    let grpc = config.sandbox.grpc_port;
    let servers = BTreeMap::from([
        ("web".to_string(), 5733u16),
        ("http".to_string(), 8001u16),
        ("dind".to_string(), 2375u16),
        ("agent".to_string(), grpc),
        ("proxy".to_string(), SANDBOX_PREVIEW_PROXY_PORT),
        ("api".to_string(), 3000u16),
    ]);

    let admitted = admitted_server_ports(servers, &config, "task-1");

    assert_eq!(
        admitted,
        BTreeMap::from([("web".to_string(), 5733u16), ("api".to_string(), 3000u16)]),
        "8001, dind (2375), the gRPC port and the preview proxy port must be filtered out"
    );
}

/// Every container in the pod shares one network namespace, so a declared
/// server admitted on the proxy's port would lose the bind and die with
/// `EADDRINUSE`, while its route still reaches the proxy — which refuses its
/// own port.
///
/// The filter and the proxy must read one rule, not two lists.
#[test]
fn a_declared_server_on_the_preview_proxy_port_is_refused() {
    let config = test_config();
    let servers = BTreeMap::from([("preview".to_string(), SANDBOX_PREVIEW_PROXY_PORT)]);

    assert!(admitted_server_ports(servers, &config, "task-1").is_empty());
}

#[test]
fn repo_config_preserves_k8s_server_projection() {
    let mut plan = test_plan();
    plan.repo_config.servers = Some(vec![ServerConfig {
        name: "web".into(),
        port: Port::new(3000),
        start_commands: vec!["run".into()],
        env_vars: serde_json::from_value(serde_json::json!({"KEY": "value"})).expect("env vars"),
        auto_reload: true,
        docker_image: Some("image".into()),
        label: Some("Web".into()),
        ..Default::default()
    }]);
    let encoded = build_repo_config_json(&plan).expect("config");
    let value: serde_json::Value = serde_json::from_str(&encoded).expect("json");
    assert_eq!(
        value["servers"],
        serde_json::json!([{
            "name": "web", "port": 3000, "startCommands": ["run"],
            "autoReload": true, "label": "Web"
        }])
    );
}

#[test]
fn server_container_sources_server_env_in_startup() {
    let config = test_config();
    let plan = test_plan();

    let containers = build_server_containers(&config, "task-1", &plan).unwrap();
    let web = containers
        .iter()
        .find(|container| container.name == "web-server")
        .unwrap();
    let args = web.args.as_ref().unwrap();
    assert_eq!(args.len(), 1);
    let startup = &args[0];
    assert!(
        startup.contains(". /opt/moatless/data/server.env"),
        "expected server.env sourcing in startup command, got: {startup}"
    );
    assert!(
        startup.contains(".workspace-ready"),
        "expected workspace-ready gate in startup command, got: {startup}"
    );
    let env = web.env.as_ref().unwrap();
    assert!(env.iter().any(|var| {
        var.name == "WORKSPACE_DIR" && var.value.as_deref() == Some("/opt/moatless/workspace")
    }));
    assert!(env.iter().any(|var| {
        var.name == "WORKSPACE_PATH" && var.value.as_deref() == Some("/opt/moatless/workspace")
    }));
}

#[test]
fn server_container_treats_empty_repo_image_as_unset() {
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.docker_image = Some(String::new());

    let containers = build_server_containers(&config, "task-1", &plan).unwrap();
    let web = containers
        .iter()
        .find(|container| container.name == "web-server")
        .unwrap();
    assert_eq!(web.image.as_deref(), Some("sandbox:latest"));
}

#[test]
fn server_container_treats_empty_server_image_as_unset() {
    let config = test_config();
    let mut plan = test_plan();
    if let Some(ref mut servers) = plan.repo_config.servers {
        servers[0].docker_image = Some(String::new());
    }

    let containers = build_server_containers(&config, "task-1", &plan).unwrap();
    let web = containers
        .iter()
        .find(|container| container.name == "web-server")
        .unwrap();
    assert_eq!(web.image.as_deref(), Some("sandbox:latest"));
}
