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
| `CHAN`    | Channel integrations            |

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

## File Naming

Spec files use descriptive names:
- `core-lifecycle.md` — Cross-domain happy-path flow
- `task-crud.md` — Task management
- `sandbox-operations.md` — Sandbox provisioning and lifecycle
- `agent-execution.md` — Agent modes, stop, error recovery
- `file-operations.md` — Workspace file operations
- `repository.md` — Repository management

## Standalone Scenarios

Scenarios that don't depend on chain state (typically error cases) go in a **Standalone** section at the end of the file. These map to individual test cases that create and tear down their own resources.
