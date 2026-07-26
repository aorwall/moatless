# Spec Authoring Guide

This directory contains testable specifications for the Moatless Vibe service. Each spec file defines scenarios in plain language that map directly to automated E2E tests.

## Spec ID Convention

Every scenario has a permanent ID in the format `{DOMAIN}-{NUMBER}` or `{DOMAIN}-{NUMBER}.{SUB}`.

### Domain Prefixes

| Prefix    | Domain                          |
|-----------|---------------------------------|
| `TASK`    | Task lifecycle and CRUD         |
| `SANDBOX` | Sandbox provisioning and management |
| `SERVER`  | Preview server lifecycle        |
| `AGENT`   | Agent execution and conversation |
| `FILE`    | File and workspace operations   |
| `REPO`    | Repository management           |
| `WS`      | Workspaces and git config sync  |
| `CHAN`    | Channel integrations            |
| `AUTH`    | Authentication and access control |

### ID Rules

- IDs are **permanent** — never renumber or reuse a deleted ID
- Sub-scenarios use dot notation: `SANDBOX-01.1`, `SANDBOX-01.2`
- IDs are unique across the entire spec suite (not just per file)
- Reference IDs in test code comments: `// TASK-01: Create task with message`

## Chain-Based Organization

Scenarios are grouped into **chains** — serial test flows where each scenario depends on the outcome of its parent. Each chain maps directly to one `test.describe.serial` block in the test runner.

### Why Chains?

- E2E tests for provisioned environments are expensive (minutes per sandbox)
- Chains reuse a single sandbox across related scenarios
- The dependency tree makes preconditions explicit
- Each chain is independently runnable

### Chain Rules

1. The first scenario in a chain creates the shared resource (usually a task + sandbox)
2. Child scenarios inherit all parent outcomes as implicit preconditions
3. Scenarios within a chain run top-to-bottom (serial)
4. Chains are independent of each other (parallel-safe)

## Scenario Format

Every scenario follows the Given / When / Then structure:

```markdown
### DOMAIN-NN: Short descriptive title

One-line description of what this scenario verifies and why it matters.

* **Given** precondition (reference parent scenario ID if dependent)

* **When** the action under test

* **Then** the expected outcome
  * **And** additional assertions
  * **And** further assertions
```

### Format Rules

- **Given**: Reference parent ID for chain dependencies, e.g., "TASK-01 has completed"
- **When**: Single action — a user interaction or a system event, described in plain language
- **Then**: Observable outcomes — what the user sees, what state changes
- One scenario = one test case (but may have multiple assertions)

## Writing Style

Specs should be readable by anyone — PMs, designers, new engineers — without needing to know the API shape or infrastructure details.

- **Describe what happens, not how to call it.** Write "the user creates a task" not "POST /api/v1/tasks is called"
- **Avoid endpoint paths.** No API URLs, HTTP methods, or query parameters
- **Avoid field names.** Write "the sandbox becomes ready" not "environmentStatus transitions to ready"
- **Avoid HTTP status codes.** Write "the system returns an error indicating not found" not "the response status is 404"
- **Avoid infrastructure terminology.** No Kubernetes resource names (Deployment, PVC, IngressRoute), scaling language ("scale to 0"), or container concepts
- **Avoid SSE event strings.** Write "a turn-completed notification is received" not "a taskTurn.completed event is received"
- **Use plain-English state names.** Write "the sandbox becomes ready" not "`environmentStatus` is `ready`"

## Concept Glossary

| Concept | Meaning |
|---------|---------|
| Task | A unit of work containing a user request and agent conversation |
| Sandbox | An isolated execution environment provisioned for each task |
| Agent | The AI assistant that processes messages and makes code changes |
| Turn | One round of agent work in response to a user message |
| Preview server | A web server running the user's application inside the sandbox |
| Plan mode | A mode where the agent proposes a plan before making changes |

## Overview Tree

Each spec file starts with an overview tree showing the chain structure and scenario dependencies. The tree uses box-drawing characters:

```
## Chain N: Chain Title
├── DOMAIN-NN: Scenario title
│   ├── DOMAIN-NN.1: Sub-scenario
│   └── DOMAIN-NN.2: Sub-scenario
│       └── DOMAIN-MM: Dependent scenario
└── DOMAIN-PP: Last scenario in chain
```

The tree must match the heading structure exactly — every ID in the tree has a corresponding `###` heading below.

## Directory Structure

Specs are organized into per-domain directories. Each domain has a `00-design.md` (non-technical design overview) and numbered scenario files.

```
specs/
├── CLAUDE.md                          # This authoring guide
├── tasks/
│   ├── 00-design.md                   # Tasks + tags
│   └── 01-task-scenarios.md           # TASK-01 through TASK-11, plus TASK-02
├── sandbox/
│   ├── 00-design.md                   # Sandbox lifecycle, preview, files
│   ├── 01-sandbox-scenarios.md        # SANDBOX-01 through SANDBOX-07
│   └── 02-file-scenarios.md           # FILE-01 through FILE-05
├── agent/
│   ├── 00-design.md                   # Agent execution, messages, commands
│   └── 01-agent-scenarios.md          # AGENT-01 through AGENT-06, FILE-06
├── repositories/
│   ├── 00-design.md                   # Repository management
│   └── 01-repository-scenarios.md     # REPO-01 through REPO-07
├── workspaces/
│   ├── 00-design.md                   # Workspaces, git-declared config, overrides
│   └── 01-git-override-scenarios.md   # WS-01 through WS-08, WS-20, WS-21
├── channels/
│   ├── 00-design.md                   # Slack/webhook integrations
│   └── 01-channel-scenarios.md        # CHAN-01 through CHAN-63
├── auth/
│   ├── 00-design.md                   # Login, sessions, API keys, teams, access control
│   ├── 01-team-scenarios.md           # AUTH-01 through AUTH-08: Team management
│   └── 02-access-control-scenarios.md # AUTH-10 through AUTH-30: Scope-based access control
└── uploads/
    └── 00-design.md                   # File uploads/attachments
```

### Cross-Domain References

- **TASK-02** lives in `tasks/01-task-scenarios.md` and is referenced by `sandbox/01-sandbox-scenarios.md` as a chain prerequisite.
- **FILE-06** lives in `agent/01-agent-scenarios.md` because it verifies agent execution outcomes. The sandbox design doc notes its location.

## Standalone Scenarios

Scenarios that don't depend on chain state (typically error cases) go in a **Standalone** section at the end of the file. These map to individual test cases that create and tear down their own resources.
