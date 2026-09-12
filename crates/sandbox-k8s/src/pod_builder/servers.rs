//! The server ports a plan declares, and which of them a pod admits.

use super::*;

/// Build servers name→port map from plan.
pub fn build_servers_map(plan: &ProvisionPlan, task_id: &str) -> HashMap<String, u16> {
    let mut servers = HashMap::new();
    if let Some(ref server_configs) = plan.repo_config.servers {
        for s in server_configs {
            if let Some(port) = allocated_port(s, task_id) {
                servers.insert(s.name.clone(), port);
            }
        }
    }
    servers
}

/// Drop any server whose port collides with a reserved sandbox port, logging
/// each rejection. The retained map is the single source of truth for what gets
/// a Service port, an `IngressRoute`, and a preview `NetworkPolicy` allowance, so
/// filtering here keeps a repo from routing the edge onto infrastructure ports.
///
/// The rule is `moatless_types::sandbox::is_reserved_sandbox_port`, called
/// directly rather than copied into a local list: a second copy admitted a
/// declared server on `SANDBOX_PREVIEW_PROXY_PORT` for as long as it lagged the
/// rule, and in the shared pod netns that server loses the bind to the proxy.
pub fn admitted_server_ports(
    servers: BTreeMap<String, u16>,
    config: &Config,
    task_id: &str,
) -> BTreeMap<String, u16> {
    servers
        .into_iter()
        .filter(|(name, port)| {
            let admitted = !is_reserved_sandbox_port(*port, config.sandbox.grpc_port);
            if !admitted {
                warn!(
                    task_id,
                    server = %name,
                    port,
                    "Refusing to expose preview server on a reserved sandbox port"
                );
            }
            admitted
        })
        .collect()
}
