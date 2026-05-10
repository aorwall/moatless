---
name: dev-loop
description: "This skill should be used when implementing features, fixing bugs, or making code changes to the backend, sandbox, or frontend. Auto-triggers when working on files in backend/src/**/*.rs, sandbox/src/**/*.rs, apps/frontend/src/**/*.tsx, specs/**/*.md, or tests/**/*.ts. Also triggers on: implement feature, fix bug, add feature, deploy, build docker, test in docker, verify in k8s, dev loop, development workflow, write spec, update spec, run e2e tests, push backend, push frontend, deploy to k8s"
---

# Dev Loop — Closed Development Workflow

Every code change follows the same closed loop: **Plan -> Implement -> Spec -> Docker -> Deploy -> Verify**. This skill orchestrates the full cycle, referencing specialized skills for deep-dive details.

## Workflow Overview

### Feature Development

```
Plan (plan mode) -> Implement + Mandatory Checks
  -> Write/Update Spec + Tests -> Build Docker -> Test in Docker Compose
  -> Push + Deploy to K8s -> Verify on moatless.dev
```

### Bug Fix

```
Reproduce (Docker Compose or moatless.dev) -> Root Cause Analysis
  -> Fix + Update Spec/Tests -> Build Docker -> Test in Docker Compose
  -> Push + Deploy to K8s -> Verify on moatless.dev
```

## Phase 1: Plan & Implement

Use plan mode for non-trivial changes. Identify which codebase area is affected:

| Area | Directory | Language | Skill Reference |
|------|-----------|----------|-----------------|
| Backend API | `backend/src/` | Rust (Axum + Tonic) | `backend-dev-guidelines` |
| Sandbox runtime | `sandbox/src/` | Rust (Agent SDK) | `backend-dev-guidelines` |
| Frontend UI | `apps/frontend/src/` | React 19 + TypeScript | `frontend-dev-guidelines` |
| Shared crates | `crates/` | Rust | `backend-dev-guidelines` |

### Mandatory Checks After Implementation

Run before proceeding to the next phase:

**Rust (backend/sandbox):**
```bash
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
# or for sandbox:
cargo clippy -p moatless-sandbox -- -D warnings
cargo test -p moatless-sandbox
```

**Frontend:**
```bash
bun run type:check
bun run biome:check    # or biome:fix to auto-fix
```

## Phase 2: Write/Update Specs & Tests

Every feature or bug fix should have a corresponding spec and test. Specs and tests are maintained in lockstep.

### Spec Files

Location: `specs/` — plain-language Given/When/Then scenarios with permanent IDs.

Consult `references/spec-writing.md` for the full format guide (ID conventions, chain rules, writing style).

### Test Files

Location: `tests/api/chains/` — Playwright API tests implementing spec scenarios.

| Spec Concept | Test Implementation |
|--------------|-------------------|
| Chain | `test.describe.serial` block |
| Scenario ID | Test name: `test("TASK-01: ...")` |
| Given | `ChainContext` state from parent test |
| When | Step function in `tests/api/steps/` |
| Then | Assertions on step results |

### Mapping Convention

| Spec File | Test File(s) |
|-----------|-------------|
| `specs/task-crud.md` | `tests/api/chains/task-crud.spec.ts` |
| `specs/sandbox-operations.md` | `tests/api/chains/sandbox-lifecycle.spec.ts` |
| `specs/agent-execution.md` | `tests/api/chains/agent-*.spec.ts` |
| `specs/file-operations.md` | `tests/api/chains/file-operations.spec.ts` |
| `specs/repository.md` | `tests/api/chains/repository.spec.ts` |
| `specs/channels.md` | `tests/api/chains/channel-*.spec.ts` |

When adding a new scenario: add the spec entry first, then implement the step function, then add the test.

## Phase 3: Build & Test in Docker

Build the changed component and test in Docker Compose. Consult `references/docker-workflow.md` for detailed commands.

### Quick Reference

```bash
# Build the changed component
task build:backend          # Rust backend
task build:frontend         # Frontend (Vite + nginx)
task build:sandbox          # Sandbox images

# Start Docker Compose (full stack)
task compose:up

# Or: frontend hot-reload + Docker Compose backend
task dev

# Rebuild backend only (after code changes)
task compose:refresh-backend

# Run API E2E tests against Docker Compose
task test:api

# Run a single test chain
task test:api:single -- task-crud

# Check logs
task compose:logs           # Pretty-printed logs
docker logs moatless-backend --tail 100

# Health check all services
task compose:verify
```

### Iterate

If tests fail: check logs, fix code, rebuild the affected component, re-run tests. Reference the `docker-testing` skill for container debugging and the `troubleshooting` reference for cross-environment patterns.

## Phase 4: Deploy to K8s

Push images and deploy to the moatless cluster. Consult `references/k8s-deployment.md` for the full workflow.

### Quick Reference

```bash
# Push changed images
task push:backend           # Cross-compile + push to Docker Hub
task push:frontend          # Build + push frontend
task push:sandbox           # Build + push sandbox

# Deploy via Helm
helm upgrade vibe k8s/helm/moatless-vibe -n vibe -f k8s/helm/values-moatless.yaml

# Restart specific deployment
kubectl rollout restart deployment/moatless-vibe-backend -n vibe
kubectl rollout status deployment/moatless-vibe-backend -n vibe --timeout=300s

# Check status
task status                 # Pods + deployments
task logs                   # Backend logs
task logs:worker            # Sandbox pod logs
```

## Phase 5: Verify on Production

Verify the deployed change on moatless.dev using API calls with an auth token.

### API Verification

```bash
# Use the test API key (set up via kubernetes-testing skill)
export API_TOKEN="$TEST_API_KEY"
export BASE_URL="https://moatless.dev"

# Verify auth works and returns admin role
curl -s -D - -o /dev/null -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/v1/auth/verify" 2>&1 | grep -i "x-user"

# List repositories
curl -s -H "Authorization: Bearer $API_TOKEN" "$BASE_URL/api/v1/repositories" | jq '.[] | {id, name}'

# Create a test task
TEMPLATE_ID="<id>"
TASK_ID=$(curl -s -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Test message\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks/vibe" | jq -r '.taskId')

# Monitor task
curl -s -H "Authorization: Bearer $API_TOKEN" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq '{status, numTurns}'

# Cleanup
curl -s -X DELETE -H "Authorization: Bearer $API_TOKEN" "$BASE_URL/api/v1/tasks/$TASK_ID"
```

### Browser Verification with playwright-cli

For UI or preview changes, use `playwright-cli` with an authenticated session cookie.

#### Step 1: Get the Test API Key

```bash
# From K8s secret
export API_TOKEN=$(kubectl get secret -n vibe vibe-secrets -o jsonpath='{.data.TEST_API_KEY}' | base64 -d)
```

#### Step 2: Exchange API Key for Session Cookie

```bash
# Exchange API key for a session cookie (domain: .moatless.dev)
SESSION_COOKIE=$(curl -sD - -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  "https://moatless.dev/api/v1/auth/api-key/session" \
  | grep -i set-cookie | sed 's/.*moatless_session=//;s/;.*//')
echo "Session cookie: $SESSION_COOKIE"
```

The backend creates a session and sets `moatless_session` cookie with `Domain=.moatless.dev` (covers `*.moatless.dev` subdomains including preview URLs).

#### Step 3: Set Cookie in playwright-cli

```bash
# Open browser and set the session cookie
playwright-cli open
playwright-cli cookie-set moatless_session "$SESSION_COOKIE" \
  --domain=.moatless.dev --httpOnly --secure --sameSite=Lax
```

#### Step 4: Navigate and Verify

```bash
# Navigate to dashboard
playwright-cli goto https://moatless.dev/

# Navigate to a specific task
playwright-cli goto https://moatless.dev/tasks/$TASK_ID

# Take screenshots for visual verification
playwright-cli screenshot

# Check accessibility snapshot for element inspection
playwright-cli snapshot

# Verify a preview URL directly (requires cookie from Step 3)
playwright-cli goto https://${TASK_ID}--5173.moatless.dev/
```

#### Step 5: Save Session for Reuse (Optional)

```bash
# Save browser state (cookies + localStorage) for reuse across sessions
playwright-cli state-save moatless-session

# In a future session, restore saved state
playwright-cli state-load moatless-session
```

### Run E2E Tests Against Production

```bash
BASE_URL=https://moatless.dev GITHUB_PAT=$GITHUB_PAT task test:api
```

> **Note**: The `COOKIE_DOMAIN` is set to `.moatless.dev` in Helm values (`auth.cookieDomain`), ensuring session cookies are shared across `moatless.dev` and all `*.moatless.dev` preview subdomains.

## Component Quick Reference

| Component | Build | Push | Deploy | Verify |
|-----------|-------|------|--------|--------|
| Backend | `task build:backend` | `task push:backend` | `kubectl rollout restart deployment/moatless-vibe-backend -n vibe` | `task logs` |
| Frontend | `task build:frontend` | `task push:frontend` | `task deploy:frontend` | playwright-cli on moatless.dev |
| Sandbox | `task build:sandbox` | `task push:sandbox` | New pods use new image automatically | Create test task |

## Troubleshooting

Consult `references/troubleshooting.md` for debugging patterns across Docker Compose and K8s environments.

Key patterns:
- **Docker logs**: `docker logs moatless-backend --tail 200 | grep -i error`
- **K8s pod logs**: `kubectl logs -n vibe deployment/moatless-vibe-backend --tail=200`
- **Sandbox pod logs**: `kubectl logs -n vibe <pod> -c sandbox --tail=200`
- **Frontend browser**: Use `playwright-cli` skill to inspect moatless.dev or localhost

## Existing Skill References

| Skill | Use For |
|-------|---------|
| `docker-testing` | Docker Compose debugging, container management, preview server issues |
| `kubernetes-testing` | K8s pod management, metrics, integration tests on moatless.dev |
| `e2e-testing` | UI E2E testing patterns with MCP tools |
| `frontend-dev-guidelines` | React component patterns, TanStack Query/Router, Tailwind |
| `backend-dev-guidelines` | Axum routes, handlers, dao, gRPC, sqlx |
| `testing-guidelines` | Test strategy, mocking, tier classification |
| `playwright-cli` | Browser automation for frontend verification |
