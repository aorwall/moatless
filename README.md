# Moatless

> Moatless is an internal tool we run every day at
> [Soap Labs](https://soaplabs.com). I built it as an experiment, so
> interfaces, configuration and the database schema may change without notice,
> and there are no plans for proper releases.

Moatless runs coding agents (Claude Code, Codex and others) in on-demand
sandboxes in a Kubernetes cluster, each with one or more of your git
repositories checked out.

## How it works

| Guide | What it covers |
| --- | --- |
| [Loops and routing](docs/guides/loops.md) | Recurring agent work fired by a schedule, Slack, Linear or GitHub, and how events reach tasks |
| [Harnesses](docs/guides/harnesses.md) | The coding agents Moatless runs, bringing your own credentials, and fallback between providers |
| [Workspaces and sandboxes](docs/guides/workspaces-and-sandboxes.md) | Declaring repositories and run settings, and the pod each task runs in |
| [Orchestration](docs/guides/orchestration.md) | When to start a new task, parent and child tasks, and agents messaging each other |
| [Artifacts](docs/guides/artifacts.md) | Pages an agent publishes for reports and specs, interactive pages, and forking from one |
| [Self-improvement](docs/guides/self-improvement.md) | Feedback from agents, weekday triage into fix tasks, and memory between runs |
| [The T3 Code UI](docs/guides/t3-code.md) | The browser client, what the Moatless fork adds, and how it connects to the backend |

| Use case | What it shows |
| --- | --- |
| [PR reviews](docs/use-cases/pr-reviews.md) | An agent that reviews every pull request on a repository |
| [Moatless fixing itself](docs/use-cases/moatless-fixing-itself.md) | An alert monitor that explains alerts or starts fix tasks that open PRs |
| [On-demand admin UIs](docs/use-cases/admin-uis-with-artifacts.md) | Managing users and configuration through pages an agent builds on request |

## How we use it at Soap Labs

We are a team of six, and we ran more than 6,000 Tasks in the last month.
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
- **Software factory.** A Loop takes a ticket from refined requirement to design
  spec to implementation across several repositories, checked by tests and
  preview environments. People approve the requirement and the design, and
  merge the pull requests.
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

All of Moatless's code is written by agents, and it has not had a security
audit. There is no guarantee that it is secure. Agents run code
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
