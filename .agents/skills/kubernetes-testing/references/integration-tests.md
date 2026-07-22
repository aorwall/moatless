# Cluster integration tests

The automated way to test against a real cluster is the **env-agnostic cluster
suite** in `tests/cluster/` — a black-box Playwright suite that drives a live
deployment over HTTP. It replaces the old per-driver integration tests.

Full reference: the `e2e-testing` skill, **Cluster target** section.

## Prerequisites

- `kubectl` access to the target cluster (only for debugging — the suite itself
  is pure HTTP and needs no kubeconfig).
- `TEST_API_KEY` (`mvk_…`) in the cluster's Secret. The backend mints a
  `test-api-user` (admin) and attaches the key. The chart wires `TEST_API_KEY`
  as an optional env (`backend-deployment.yaml`).
- For the agent rung: Claude creds provisioned in the cluster.

## Running

```bash
# Read the key from the cluster (local example).
API_KEY=$(kubectl -n moatless get secret moatless-vibe-secrets \
  -o jsonpath='{.data.TEST_API_KEY}' | base64 -d)

# Smoke + lifecycle (deterministic, free).
MOATLESS_BASE_URL=http://moatless.localhost API_KEY="$API_KEY" \
  bun run test:e2e:cluster

# Full run incl. the agent building a page (needs Claude creds in the cluster).
MOATLESS_BASE_URL=https://moatless.dev API_KEY="$API_KEY" E2E_DEPTH=full \
  bun run test:e2e:cluster
```

Point `MOATLESS_BASE_URL` at any cluster's ingress — the suite is identical for
local-k8s and remote.

## Coverage

| Rung | `E2E_DEPTH` | Checks |
|------|-------------|--------|
| smoke | (default) | health, API-key auth, task CRUD |
| lifecycle | (default) | sandbox ready, preview serves, events over SSE, status |
| agent | `full` | agent edits the page to a unique marker; preview renders it |

## CI

`.github/workflows/cluster-e2e.yml` runs it against the remote cluster:
post-deploy (smoke + lifecycle) and nightly (full). local-k8s is not a CI
target — run it by hand.
