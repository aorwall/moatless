# Moatless

Moatless runs coding agents, such as Claude Code and Codex, in on-demand
sandboxes against your git repositories. A person drives a Task from the web UI,
the `moat` CLI, Slack, Linear or a GitHub pull request; a Loop drives Tasks on a
schedule or in response to events. Each Task gets its own sandbox container or
pod with the repository checked out, and the agent's work comes back as
messages, commits and pull requests.

The backend is Rust (`crates/`); it runs sandboxes as sibling Docker containers
or as Kubernetes pods. The web UI is a T3 Code client, served over
the backend's UI-RPC surface (`crates/t3code`).

## Quickstart with Docker Compose

Build the `moat` CLI from this checkout (a Rust toolchain is required):

```bash
cargo install --path crates/cli
```

Then let it check the host, write the Compose configuration and start the
stack:

```bash
moat docker preflight
moat docker init
moat docker env set CLAUDE_CODE_OAUTH_TOKEN <token>   # from `claude setup-token`
moat docker install --pull --wait
moat docker status
```

Sign in from the CLI:

```bash
MOATLESS_API_URL=http://localhost:8080 moat login
moat status
```

[docker-compose/README.md](docker-compose/README.md) covers the Compose stack
in depth: services, configuration, operations and troubleshooting.

## Kubernetes

The Helm chart in [k8s/helm/moatless](k8s/helm/moatless/README.md) installs the
backend, its sandboxes and an optional bundled Postgres. `moat k8s` generates
values and secrets for it. [k8s/local](k8s/local/README.md) runs the chart on a
single-node local cluster.

## Documentation

- [CONTEXT.md](CONTEXT.md): the glossary of domain terms (Task, Loop,
  Workspace, Sandbox, Adapter, ...).
- [docs/api-authentication.md](docs/api-authentication.md): signing in and
  calling the REST API with an API key.
- [docs/docker-builds.md](docs/docker-builds.md): building the backend and
  sandbox images.
- [docs/slack_manifest.yaml](docs/slack_manifest.yaml): a Slack app manifest for
  the Slack adapter.
- [AGENTS.md](AGENTS.md): how to build, test and contribute; coding agents read
  it, and so should you.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).
