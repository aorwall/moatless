# Source map

Use code and tests to settle behavior; documentation is a routing aid.

| Question | Owner |
|---|---|
| Pod lifecycle observation | [event.rs](../../../../crates/sandbox-k8s/src/pod_watcher/event.rs), [observed.rs](../../../../crates/sandbox-k8s/src/pod_watcher/observed.rs) |
| Health classification | [status_resolver.rs](../../../../crates/sandbox-k8s/src/status_resolver.rs), [pod_status.rs](../../../../crates/sandbox-k8s/src/pod_status.rs) |
| Pod construction, env, resources | [pod.rs](../../../../crates/sandbox-k8s/src/pod_builder/pod.rs), [containers.rs](../../../../crates/sandbox-k8s/src/pod_builder/containers.rs) |
| Sandbox runtime operations | [Kubernetes provider](../../../../crates/sandbox-k8s/src/provider.rs), [Docker provider](../../../../crates/sandbox-docker/src/provider.rs) |
| Lifecycle orchestration | [sandbox-control](../../../../crates/sandbox-control/src) |
| Workspace persistence | [storage.rs](../../../../crates/sandbox-runtime/src/storage.rs) |
| Task creation and first message | [task service](../../../../crates/tasks/src/task/service.rs) |
| Script port publishing | [edge.rs](../../../../crates/sandbox-k8s/src/edge.rs) |
| Preview ingress | [ingress.rs](../../../../crates/sandbox-k8s/src/ingress.rs) |
| CLI diagnostic output | [tasks.rs](../../../../crates/cli/src/commands/tasks.rs) |

Read the owning crate's AGENTS.md before changing its code.
