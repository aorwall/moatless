# Moatless Vibe on a local Kubernetes cluster

Run Moatless Vibe on a single-node Kubernetes cluster on your machine, using
locally built images and the chart's first-party Postgres and Redis. This is the
supported local path — no Docker Hub pull, no SOAP registry mirror, no cloud.

`values-local.yaml` in this directory is the committed local config. It contains
**no secrets** — you create the Secret by hand (step 3 below).

## Prerequisites

- **Docker Desktop 4.74+ with Kubernetes enabled in kubeadm mode.** kubeadm mode
  shares the local Docker image store, so pods can run images you built locally
  with no registry push and no `kind load`. kind mode does not share the store —
  do not use it here.
- **Apple Silicon (arm64).** The build scripts detect the host arch and build
  native arm64 images.
- Give the Docker Desktop VM enough headroom — **~16GB memory**, and keep
  **~20GB free host disk**: the sandbox image is 5-8GB and Docker.raw grows
  against the host filesystem.
- Rust cross-compile toolchain: `cargo-zigbuild` and `zig` on PATH, plus the
  Linux target's std library — **`rustup target add aarch64-unknown-linux-gnu`**
  (zig provides the linker; rustup must provide the target std).
- The sandbox image also needs a `dev-browser` checkout. Auto-detected at
  `../../stuffs/dev-browser` **relative to the repo root** — from a git worktree
  that path is wrong, so pass `--dev-browser-dir <abs-path>` explicitly.
- `kubectl` and `helm` 3.x pointed at the Docker Desktop context
  (`kubectl config use-context docker-desktop`).

## 1. Build the images (native arm64, tagged `:dev`)

```bash
task build:backend TAG=dev                                    # backend
./docker/build-app.sh --local --tag dev                       # frontend
./docker/build-sandbox.sh --local --tag dev \                 # sandbox (base + tools + runtime)
  --dev-browser-dir "$(cd ../.. && pwd)/stuffs/dev-browser"
```

Use `docker/build-*.sh --local` (single-arch, loads into the local store), **not**
`task build:frontend` / `task build:sandbox` — those build multi-arch and *push*,
which never lands the image locally. `build-sandbox.sh --local` builds the base,
the sandbox-tools image, and the runtime image in one pass, all tagged `:dev`.

Docker Desktop's kubeadm mode shares the image store, so nothing needs pushing.

> The images `values-local.yaml` expects: `aorwall/moatless-frontend:dev`,
> `aorwall/moatless-backend:dev`, `aorwall/moatless-sandbox:dev`. `pullPolicy` is
> `IfNotPresent` for all three, so Kubernetes uses the local build.

## 2. Traefik + namespace

Traefik provides the `IngressRoute` CRDs the app routing (and the backend's
per-sandbox preview routes) depend on.

```bash
kubectl create namespace moatless
helm repo add traefik https://traefik.github.io/charts && helm repo update
helm install traefik traefik/traefik -n moatless \
  --set service.type=LoadBalancer --set ports.web.exposedPort=80
```

## 3. Secrets (never in a committed values file)

```bash
kubectl -n moatless create secret generic moatless-vibe-secrets \
  --from-literal=DATABASE_URL='postgres://moatless:moatless@postgres:5432/moatless' \
  --from-literal=SECRETS_ENCRYPTION_KEY="$(openssl rand -base64 32)" \
  --from-literal=TASK_TOKEN_SECRET="$(openssl rand -hex 32)"
```

The deployment expects the Secret to be named exactly `moatless-vibe-secrets`
(`{release}-secrets`, and the release must be `moatless-vibe` — see step 4).
`values-local.yaml` keeps `secrets.create: false` so Helm does not fight this
hand-made Secret.

`DATABASE_URL` here matches the bundled Postgres Service exactly; the chart also
sets the backend's `DATABASE_URL` from `postgres.enabled`, so the two agree.
`SECRETS_ENCRYPTION_KEY` and `TASK_TOKEN_SECRET` are both mandatory — the backend
refuses to boot without the first, and every sandbox provision fails without the
second.

## 4. Install

**The release name must be `moatless-vibe`.** The backend hardcodes the sandbox's
callback host as `moatless-vibe-backend`
(`backend/src/sandbox/k8s/pod_builder.rs`), which is `{release}-backend` only for
this release name. Install it under any other name and every sandbox fails to
reach the backend with a gRPC `dns error`.

```bash
helm install moatless-vibe ./k8s/helm/moatless-vibe -n moatless -f k8s/local/values-local.yaml
```

## 5. Verify boot

```bash
# The real boot check — a bad DATABASE_URL does NOT crash the pod (the backend
# logs and continues, /health still returns 200), so a green pod is not proof.
# On a cold start the backend retries the DB connection for ~30s to ride out a
# Postgres that is still initializing.
kubectl -n moatless logs deploy/moatless-vibe-backend | grep "Database migrations complete"

# App reachable through Traefik as the auto-created local-user.
curl -H 'Host: vibe.localhost' localhost/api/v1/auth/me      # 200
```

Open `http://vibe.localhost` and you are in.

## 6. First run — credential, repo, task

- **Login needs no OAuth.** `auth.provider` is empty (the default), so every
  request resolves to `user_id: local-user`, `role: admin`; that row is
  auto-created at boot.
  - Known cosmetic issue: dev-mode `/api/v1/auth/me` returns `user: null`, so the
    frontend hides admin-only *UI* routes. Backend authorization is unaffected.
- **Add the agent credential in the UI:** Settings → Secrets →
  `CLAUDE_CODE_OAUTH_TOKEN`. It is read only from the encrypted DB (never from
  chart values), so it needs a working `SECRETS_ENCRYPTION_KEY`.
- **Create a repository without a provider.** The seeded "Moatable" repo sets
  `provider: Github`, which trips the GitHub-token preflight and errors the task
  before any pod is created — even for a public, anonymously clonable repo. Create
  one omitting `provider` and passing `servers` explicitly:

  ```bash
  curl -X POST -H 'Host: vibe.localhost' -H 'Content-Type: application/json' \
    -H 'X-User-Id: local-user' -H 'X-User-Role: admin' \
    localhost/api/v1/repositories \
    -d '{"name":"local-smoke","remoteUrl":"https://github.com/aorwall/moatable","defaultBranch":"main","kind":"remote","servers":[]}'
  ```

  The clone URL field is `remoteUrl` (not `git_url`), and `kind` is required. Omit
  `provider` so the GitHub-token preflight is skipped. `moat repos create` cannot
  do this — its `--provider` flag defaults to `github` — and by default `moat`
  targets the hosted instance, not this local cluster.

- **Create the task** (starts the sandbox; add a message to also run the agent —
  which needs `CLAUDE_CODE_OAUTH_TOKEN` from the step above):

  ```bash
  RID=<repository id from the create response>
  curl -X POST -H 'Host: vibe.localhost' -H 'Content-Type: application/json' \
    -H 'X-User-Id: local-user' -H 'X-User-Role: admin' \
    localhost/api/v1/tasks -d "{\"name\":\"smoke\",\"repositoryId\":\"$RID\"}"

  kubectl -n moatless get pods    # moatless-ws-<task-id> reaches 1/1 Running
  curl -H 'Host: vibe.localhost' -H 'X-User-Id: local-user' \
    localhost/api/v1/tasks/<task-id>   # environmentStatus: ready
  ```

  A ready sandbox has cloned the repo into `/opt/moatless/workspace/<repo>` and
  connected back to the backend over gRPC.

## Why these settings (gotchas)

| Setting | Why |
|---|---|
| `image.pullPolicy` / `backend.image.pullPolicy: IfNotPresent`, tag `dev` | Chart defaults both to `Always` + `latest`, which forces a Docker Hub pull and silently ignores locally built images. |
| `kubernetes.nodeStorageJanitor.enabled: false` | Default janitor image is a private SOAP ECR ref → guaranteed `ImagePullBackOff`. |
| `kubernetes.imagePrepull.enabled: false` | Pre-pulls the full 5-8GB sandbox image with `pullPolicy: Always`. |
| `kubernetes.sandboxStorage.defaultMode: ephemeral` | Default `s3` needs a real bucket; ephemeral avoids per-task PVC churn. |
| `kubernetes.sandboxSysctls` — drop `tcp_keepalive_*` | Some kubelets reject them as unsafe → block sandbox pod admission. |
| `networkPolicy.sandbox.enabled: false` | Expects a namespace literally named `traefik`; locally Traefik lives in `moatless`. |
| `proxy.domain: preview.localhost`, `protocol: http`, `traefikEntryPoint: web` | macOS resolves arbitrary `*.localhost` → `127.0.0.1`. No dnsmasq, no nip.io. |
| `proxy.traefikIngressRoute.enabled: true` | Renders the app's Traefik `IngressRoute` CRDs (locally Traefik *is* the router). |
| `ingress.enabled: false`, `proxyIngress.enabled: false` | Both target a Service named `traefik:80` in-namespace — a cloud shape where an ALB fronts Traefik. Locally Traefik is the controller, so they are circular. |
| `ingress.hosts` **kept despite `enabled: false`** | Trap: the IngressRoute templates read the app host from `ingress.hosts[0].host` regardless of the enable flag. Omit it and all 7 routes fall back to `vibe.example.com` and match nothing. |
| `postgres.enabled: true`, `redis.enabled: true` | Uses the chart's first-party Postgres/Redis templates. |

## Notes

- **`task deploy` passes no values file** (`helm upgrade` with only
  `--set image.tag=...`), so it renders chart defaults. It is not the local path —
  use the `helm install ... -f k8s/local/values-local.yaml` above.
- **Docker Desktop Kubernetes can reset on restart** on some versions. If the
  cluster comes back empty, re-run steps 2-4 (the images are still in the store).
