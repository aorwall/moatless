# Moatless

> Moatless is an internal tool we run every day at
> [Soap Labs](https://soaplabs.com). I built it as an experiment, so
> interfaces, configuration and the database schema may change without notice,
> and there are no plans for proper releases.

Moatless runs coding agents (Claude Code, Codex and others) in on-demand
sandboxes in a Kubernetes cluster, each with one or more of your git
repositories checked out.

## How it works

A **Task** is one piece of agent work: a conversation with a coding agent
(Claude Code, Codex, Pi and others), started by a person or by a Loop. It runs
in a **Workspace**, which declares the repositories to check out, the image,
setup commands, secrets and dev-server scripts. Each Task gets its own
**Sandbox**: a Kubernetes pod with the Workspace materialized on disk. Sandboxes
stop when idle and resume where they left off.

## What an agent can do from its sandbox

- **Use the `moat` CLI.** Every harness already runs shell commands, so the
  backend is reached through one CLI rather than per-harness MCP servers: reply
  in Slack or Linear, open a pull request, publish an Artifact, write to Memory,
  run a long build.
- **Run a preview.** An agent starts the app's dev server as a Script and gets a
  preview URL, so you can open the running app in the web UI next to the
  conversation, as on a vibe-coding platform.
- **Start and talk to other Tasks.** An agent splits work with
  `moat tasks create`, each child in its own sandbox, and the Tasks message each
  other with `moat send`. A child reports back to its parent when it is done.
- **Artifacts.** HTML pages that outlive the sandbox. They can be interactive: a
  person can fill in a form, pick options, leave comments or edit values on the
  page. What they enter is stored with the Artifact, and the agent can read it
  and be notified when it is saved.
- **Run on events.** A Loop, declared in a `LOOP.md`, starts Tasks on a
  schedule or when something happens in GitHub, Slack or Linear, and the Task
  replies on the same thread.

## How we use it at Soap Labs

We are a team of six. In 2026 we have run about 18,000 Tasks, growing from under
100 a month in January to more than 6,000 in the last month.
On a typical working day about 17 Tasks have a running sandbox at any moment,
peaking at over 50, and roughly half of them were started by Loops and bots
rather than people.
At least one sandbox is running at any time of day or night.

- **Non-developers.** People outside engineering refine requirements and do real
  development: they ask for a change, follow the agent's work in the T3 Code
  interface and check the result in a running preview of the app.
- **Linear tickets.** Mention or assign the agent on a ticket and a triage Loop
  reads it. It answers the question, asks what is unclear, or does the work and
  opens the pull requests.
- **Software factory.** We are experimenting with a Loop that takes a ticket
  from refined requirement to design spec to implementation across several
  repositories, checked by tests and preview environments.
- **Alert triage.** One Loop per alert channel and environment reads every alert
  in Slack, checks the logs, traces and metrics behind it, and reproduces it
  outside production. It then proposes a fix or opens a pull request, and
  escalates to another channel when an alert is more serious than its current
  one.
- **Scheduled maintenance.** Loops run recurring checks and chores on
  a timer: health and cost reports, security reviews, keeping docs and
  guidance current, and paying down test and lint debt.
- **Pull request review.** A review Loop reviews each pull request against
  that repository's own rules with the
  [`pr-review` skill](plugins/moatless-skills/skills/pr-review/SKILL.md), and
  approves once no finding holds it and CI is green.

## Getting started

Follow the [k8s-install skill](.agents/skills/k8s-install/SKILL.md) to install
Moatless with the Helm chart in
[k8s/helm/moatless](k8s/helm/moatless/README.md). It covers a local cluster in
Docker Desktop, and AKS or EKS, which need DNS, TLS and Traefik for the app and
its preview URLs. The skill is written for an agent, so you can point Claude
Code or Codex at this repository and ask it to install Moatless for you.

## Security

Moatless is largely vibe-coded: most of it is written by agents, and it has not
had a security audit. There is no guarantee that it is secure. Agents run code
you did not write, with your repositories and every credential you give a
Workspace. Run it in isolation: its own cluster or cloud account, credentials
scoped to what the agents need and that you can afford to lose, and nothing
exposed to the internet that you have not put authentication in front of.

Each sandbox is its own pod or container and runs as a non-root user with its
Linux capabilities dropped. The Helm chart by default limits its network to
HTTPS out (no private address ranges). Sandboxes hold no GitHub token or model
API key: git, `gh`, Claude Code and Codex traffic goes through the backend,
which refuses GitHub writes outside the Workspace's repositories. Secrets are
encrypted at rest.

You can tighten it in the Helm chart: run sandboxes in their own namespace with
quotas, and limit their outbound traffic with an egress allowlist or proxy.
These are off by default. You can also loosen it, for example by giving a
Workspace a Kubernetes service account or Docker access, and a careless setting
there can give agents access to your whole cluster. Keep Moatless well away from
any production workload.

## Status and support

This is shared in case it is useful to others. There is no promise of support, a
stable API or a migration path between versions, and pull requests might be
ignored.

The name comes from [moatless-tools](https://github.com/aorwall/moatless-tools),
an earlier research project I built on agent-based code editing. This repository
is a separate codebase: it runs existing agent harnesses rather than its own
agent.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

## Third-party software and trademarks

The sandbox images install third-party agent CLIs unmodified from their
publishers, among them Claude Code (proprietary, © Anthropic PBC; use is
subject to Anthropic's Legal Agreements, and each user signs in with their own
credentials) and OpenAI Codex (Apache-2.0). The web UI is an unofficial fork of
T3 Code (MIT, © 2026 T3 Tools Inc.). [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
lists each component and its license.

Claude Code is a trademark of Anthropic PBC, Codex of OpenAI, and T3 Code of
T3 Tools, Inc. Moatless is not affiliated with, endorsed by or sponsored by any
of them.
