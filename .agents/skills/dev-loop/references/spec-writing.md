# Spec Writing Guide

Specs define testable scenarios in plain language. Each spec maps directly to automated E2E tests.

## Spec ID Convention

Format: `{DOMAIN}-{NUMBER}` or `{DOMAIN}-{NUMBER}.{SUB}`

### Domain Prefixes

| Prefix | Domain |
|--------|--------|
| `TASK` | Task lifecycle and CRUD |
| `SANDBOX` | Sandbox provisioning and management |
| `SERVER` | Preview server lifecycle |
| `AGENT` | Agent execution and conversation |
| `FILE` | File and workspace operations |
| `REPO` | Repository management |
| `CHAN` | Channel integrations |

### ID Rules

- IDs are **permanent** — never renumber or reuse a deleted ID
- Sub-scenarios use dot notation: `SANDBOX-01.1`, `SANDBOX-01.2`
- IDs are unique across the entire spec suite
- Reference IDs in test code comments: `// TASK-01: Create task with message`
- To find the next available ID, check existing spec files for the highest number in that domain

## Scenario Format

Every scenario follows Given / When / Then:

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
- **When**: Single action — a user interaction or system event, described in plain language
- **Then**: Observable outcomes — what the user sees, what state changes
- One scenario = one test case (may have multiple assertions)

## Chain-Based Organization

Scenarios group into **chains** — serial test flows where each scenario depends on its parent.

### Why Chains?

- E2E tests with provisioned sandboxes are expensive (minutes per sandbox)
- Chains reuse a single sandbox across related scenarios
- The dependency tree makes preconditions explicit
- Each chain is independently runnable and parallel-safe

### Chain Rules

1. First scenario creates the shared resource (usually a task + sandbox)
2. Child scenarios inherit all parent outcomes as implicit preconditions
3. Scenarios within a chain run top-to-bottom (serial)
4. Chains are independent of each other (parallel-safe)

### Overview Tree

Each spec file starts with a tree showing chain structure:

```markdown
## Chain 1: Task CRUD
├── TASK-01: Create task with message
│   ├── TASK-01.1: Task appears in listing
│   ├── TASK-01.2: Task-created notification emitted
│   └── TASK-03: Get task by ID
└── TASK-09: Delete task
```

The tree must match the heading structure exactly.

## Writing Style

Specs must be readable by anyone — PMs, designers, new engineers — without knowing API shapes or infrastructure details.

### Do

- Describe **what happens**, not how to call it: "the user creates a task"
- Use plain-English state names: "the sandbox becomes ready"
- Focus on observable behavior: "a turn-completed notification is received"

### Do Not

- Use endpoint paths, HTTP methods, or query parameters
- Use field names: NOT "environmentStatus transitions to ready"
- Use HTTP status codes: NOT "the response status is 404" — write "error indicating not found"
- Use infrastructure terminology: No Kubernetes terms (Deployment, PVC, IngressRoute)
- Use SSE event strings: NOT "taskTurn.completed event" — write "turn-completed notification"

## Standalone Scenarios

Error cases and scenarios that don't depend on chain state go in a **Standalone** section at the end of the file. These create and tear down their own resources.

## Mapping Specs to Tests

### File Naming

| Spec | Test File |
|------|-----------|
| `specs/task-crud.md` | `tests/api/chains/task-crud.spec.ts` |
| `specs/sandbox-operations.md` | `tests/api/chains/sandbox-lifecycle.spec.ts` |
| `specs/agent-execution.md` | `tests/api/chains/agent-execution-flow.spec.ts` + `agent-plan-mode.spec.ts` + `agent-stop-recovery.spec.ts` |

### Test Structure

```typescript
test.describe("Chain: Task CRUD", () => {
    test.describe.configure({ mode: "serial" });
    const ctx = new ChainContext();

    // TASK-01: Create task with message
    test("TASK-01: Create task with message", async ({ api }) => {
        const result = await createTaskWithMessage(api, ctx.repositoryId!, "message");
        expect(result.task.status).toBe("open");
        ctx.taskId = result.taskId;
    });

    // TASK-01.1: Task appears in listing
    test("TASK-01.1: Task appears in listing", async ({ api }) => {
        const result = await listTasks(api);
        expect(result.items.some((t) => t.taskId === ctx.taskId)).toBe(true);
    });
});
```

### Step Functions

Step functions live in `tests/api/steps/{domain}.steps.ts`. Each function performs one action and returns a result — no assertions inside step functions.

```typescript
export async function createTaskWithMessage(
    api: ApiClient,
    repositoryId: string,
    message: string,
): Promise<{ taskId: string; task: TaskResponse }> {
    const task = await api.createVibeTask({ templateId: repositoryId, message });
    return { taskId: task.taskId, task };
}
```

## Adding a New Scenario

1. **Choose the domain prefix** and find the next available ID number
2. **Add to the spec file**: overview tree + scenario heading with Given/When/Then
3. **Create/update step function** in `tests/api/steps/{domain}.steps.ts`
4. **Add test** in the appropriate chain file in `tests/api/chains/`
5. **Run tests** to verify: `task test:api:single -- {chain-name}`

## Concept Glossary

| Concept | Meaning |
|---------|---------|
| Task | A unit of work containing a user request and agent conversation |
| Sandbox | An isolated execution environment provisioned for each task |
| Agent | The AI assistant that processes messages and makes code changes |
| Turn | One round of agent work in response to a user message |
| Preview server | A web server running the user's application inside the sandbox |
| Plan mode | A mode where the agent proposes a plan before making changes |
