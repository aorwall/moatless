---
name: k8s-install
description: "This skill should be used when the user asks to 'install moatless vibe', 'deploy moatless to kubernetes', 'set up moatless on AKS', 'set up moatless on EKS', 'bootstrap moatless cluster', 'install moatless helm chart', or mentions spinning up a fresh Moatless Vibe installation. It drives a preflight → init → secrets → install → verify flow using the `moat k8s` subcommands."
---

# Moatless Vibe Kubernetes Installer

Install Moatless Vibe into a Kubernetes cluster with a guided five-step flow. The CLI (`moat k8s …`) does the actual work; this skill orchestrates ordering, interprets JSON output, branches per cloud provider (AKS, EKS), and guides the user through prerequisite installation.

## When to trigger

Trigger on phrases like "install moatless on kubernetes", "deploy moatless to my EKS cluster", "set up moatless vibe on AKS", or any bootstrap request where the target is a Kubernetes cluster.

Do **not** trigger for: upgrading an existing install (use `moat k8s upgrade --tag <tag>`), deploying a new image build (use `task deploy`), or Docker Compose installs.

## Prerequisites the user must have

Before starting, confirm the user has:

- `kubectl` authenticated against the target cluster (`kubectl config current-context` works)
- `helm` 3.x installed
- A domain they control for the main app and another (or wildcard) for sandbox previews
- An Anthropic API key (required)
- Cluster admin or namespace-admin on the target namespace
- Optional: GitHub OAuth app or Auth0 tenant if they want SSO; a PostgreSQL URL and Redis URL if they want external data stores

If any piece is missing, pause the workflow and ask before proceeding — do not guess.

## The five-step workflow

Execute these in order. Each step has a single `moat k8s …` command; read its JSON output (`--json`) to decide what to do next.

### Step 1 — Preflight

```bash
moat k8s preflight --namespace <ns> --json
```

Reads the cluster and reports `checks[]` plus a derived `provider` (`aks`, `eks`, or `generic`). Interpret:

- Every `check.ok == true` → green light, continue.
- `check.ok == false` → surface the `check.hint` verbatim and ask the user to fix or let the skill help. Never proceed past a failed tooling check (kubectl/helm missing).
- Provider detection drives Step 2. If `generic`, ask the user whether they know which cloud they are on so defaults can be picked correctly.

Common failures and the right response:

| Failure | Response |
|---|---|
| Ingress controller absent on AKS | Offer to enable web app routing (`az aks approuting enable`). See `references/provider-aks.md`. |
| Ingress controller absent on EKS | Guide installation of the AWS Load Balancer Controller. See `references/provider-eks.md`. |
| No default StorageClass | On AKS: `managed-csi` is usually present; on EKS: install the EBS CSI driver. |
| Traefik CRD missing | Moatless uses IngressRoute CRDs from Traefik for per-sandbox proxy routes. Follow `references/prerequisites.md`. |
| cert-manager CRD missing | Only required if terminating TLS in-cluster. Skip for AKS web app routing + Azure Front Door. |

### Step 2 — Init values

```bash
moat k8s init --provider <aks|eks|generic> --domain <app.example.com> --proxy-domain <sandbox.example.com> --namespace <ns>
```

Interactive prompts fill remaining fields (tag, auth provider, storage backend). Writes `values.local.yaml` to cwd. Before running:

- Confirm the domain and proxy-domain with the user. The proxy domain must be a separate host (typically a wildcard) because sandbox preview URLs use `<taskId>--<port>.<proxy-domain>`.
- If the user wants GitHub SSO, collect the OAuth app callback URL beforehand: `https://<app-domain>/api/auth/github/callback`.

After the file is written, skim `values.local.yaml` and call out anything the user should customise (resource sizes, sandbox storage, monitoring).

### Step 3 — Secrets bootstrap

**Strongly prefer a `.env.local` file.** Before running the bootstrap command, ask the user to create `.env.local` at the repo root with the required credentials — see `references/secrets.md` for the template. Do **not** ask the user to paste secrets into the chat. Chat history is stored, rendered, and often synced to remote transcripts; anything pasted there can leak.

When posting secrets in chat is the only option (user insists, no shell access, etc.), call it out explicitly before proceeding: "Pasting secrets in chat works but is less secure than `.env.local`. Keys may show up in logs or transcripts. Proceed anyway?"

```bash
moat k8s secret bootstrap --namespace <ns>
```

Collection priority (highest wins, no silent overwrites):
1. Existing `Secret/vibe-secrets` keys already in the cluster
2. Keys found in `.env.local` / `.env` / `backend/.env` at repo root — the CLI will prompt before reading any value (preferred path)
3. Interactive masked prompts for whatever remains — only for keys the user couldn't put in a `.env` file

The CLI **never** reads `std::env` — process environment is noisy and unsafe to auto-consume. The only two safe paths are a `.env` file on disk or a masked terminal prompt.

Pair flags:
- `moat k8s secret list` — show which keys are set vs missing (values masked)
- `moat k8s secret set <KEY>` — one-off change, masked prompt
- `moat k8s secret pull-token` — creates `ghcr-secret` (dockerconfigjson) from a prompted GitHub PAT
- `--env-file <path>` — use a `.env` file outside the repo root

After bootstrap, confirm required keys are present with `moat k8s secret list --json` and verify both `DATABASE_URL` and `SECRETS_ENCRYPTION_KEY` appear as `source: cluster`. The backend refuses to start without `SECRETS_ENCRYPTION_KEY`. `ANTHROPIC_API_KEY` is **not** an install-time secret — users provide their own Claude credentials per task at runtime.

### Step 4 — Install

```bash
moat k8s install --namespace <ns> --release moatless-vibe --values values.local.yaml
```

Runs `helm dependency update` then `helm upgrade --install`. For cautious users, pass `--dry-run` first. Add `--wait` to block until rollout completes (10 minute timeout).

If `helm upgrade --install` fails, the JSON report contains `stderr`. Common failures:
- `rendered manifests contain a resource that already exists` → a previous broken install; ask user to `helm uninstall` first or pass a new release name.
- `ImagePullBackOff` once pods start → `ghcr-secret` missing or wrong; run `moat k8s secret pull-token`.
- `PVC pending` → StorageClass mismatch; edit `values.local.yaml` `kubernetes.sandboxStorage.storageClass` and re-run install.

### Step 5 — Verify

```bash
moat k8s status --namespace <ns> --release moatless-vibe --json
```

`all_healthy: true` means every deployment is at desired replicas and no pods are stuck. If not, the `deployments[]` entries and `pending_pods[]` list tell you what to look at. Suggest `kubectl -n <ns> describe pod <name>` or `kubectl -n <ns> logs deploy/<name>` for the specific resource.

Once healthy, fetch the ingress address from `ingresses[].address` and give the user the URL they just installed.

## Provider branching

After Step 1 detects the provider, load the matching reference file before any follow-up work:

- **AKS** → read `references/provider-aks.md` (web app routing, Azure monitor operator, sysctls via DaemonSet)
- **EKS** → read `references/provider-eks.md` (AWS LB Controller, EBS CSI, IRSA for S3)

## Interpreting `--json` outputs

Every `moat k8s` command supports `--json` and emits a machine-readable report. Always pass `--json` when the flow is scripted; parse the report and branch, rather than grepping human output.

Key fields by command:

- `preflight`: `checks[]` (`.ok`, `.hint`), `provider`, `ingress_controllers[]`, `storage_classes[]`
- `secret list`: `keys[]` (`.name`, `.source`: cluster|env|missing)
- `install`: `success`, `stderr` (on failure)
- `status`: `all_healthy`, `deployments[]`, `pending_pods[]`, `ingresses[]`

## Safe retry behaviour

All commands are idempotent:
- `init` overwrites `values.local.yaml` — confirm with the user before re-running
- `secret bootstrap` merges into the existing Secret, never erases keys unless `--overwrite` is set
- `install` is `helm upgrade --install`, safe to re-run
- `uninstall --purge` requires a confirmation prompt in interactive mode

If a step fails, re-run **that step**, not the whole flow. The cluster state from earlier steps survives.

## Additional Resources

### Reference files

- **`references/preflight-checks.md`** — what each preflight check means and how to remediate
- **`references/values-schema.md`** — required/optional values, provider nuances
- **`references/provider-aks.md`** — AKS-specific gotchas
- **`references/provider-eks.md`** — EKS-specific gotchas
- **`references/secrets.md`** — env var catalog and OAuth app setup
- **`references/prerequisites.md`** — Traefik, cert-manager, Redis install snippets
- **`references/troubleshooting.md`** — common failures → diagnosis

### Examples

- **`examples/values-aks-minimal.yaml`** — smallest working AKS config
- **`examples/values-eks-minimal.yaml`** — smallest working EKS config
- **`examples/vibe-secrets.yaml`** — annotated Secret template
