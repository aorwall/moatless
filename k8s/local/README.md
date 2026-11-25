# Moatless on a local Kubernetes cluster

Run Moatless on a single-node Kubernetes cluster on your machine, using
locally built images and the chart's first-party Postgres. This is the
supported local path — no Docker Hub pull, no private registry mirror, no cloud.

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
./docker/build-sandbox.sh --local --tag dev \                 # sandbox (base + tools + runtime)
  --dev-browser-dir "$(cd ../.. && pwd)/stuffs/dev-browser"
```

Use `docker/build-*.sh --local` (single-arch, loads into the local store), **not**
`task build:sandbox` — it builds multi-arch and *pushes*, which never lands the
image locally. `build-sandbox.sh --local` builds the base,
the sandbox-tools image, and the runtime image in one pass, all tagged `:dev`.

Docker Desktop's kubeadm mode shares the image store, so nothing needs pushing.

> The images `values-local.yaml` expects: `aorwall/moatless-backend:dev` and
> `aorwall/moatless-sandbox:dev`. `pullPolicy` is `IfNotPresent` for both, so
> Kubernetes uses the local build. The T3 Code UI image, `aorwall/moatless-t3`,
> comes from Docker Hub.

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
kubectl -n moatless create secret generic moatless-secrets \
  --from-literal=DATABASE_URL='postgres://moatless:moatless@postgres:5432/moatless' \
  --from-literal=SECRETS_ENCRYPTION_KEY="$(openssl rand -base64 32)" \
  --from-literal=TASK_TOKEN_SECRET="$(openssl rand -hex 32)" \
  --from-literal=SESSION_JWT_SECRET="$(openssl rand -hex 32)" \
  --from-literal=ADMIN_PASSWORD='choose-a-strong-password'
```

The deployment expects the Secret to be named exactly `moatless-secrets`
(`{release}-secrets` for the release `moatless` installed in step 4).
`values-local.yaml` keeps `secrets.create: false` so Helm does not fight this
hand-made Secret.

`DATABASE_URL` here matches the bundled Postgres Service exactly; the chart also
sets the backend's `DATABASE_URL` from `postgres.enabled`, so the two agree.
`SECRETS_ENCRYPTION_KEY` and `TASK_TOKEN_SECRET` are both mandatory — the backend
refuses to boot without the first, and every sandbox provision fails without the
second.

`SESSION_JWT_SECRET` signs session cookies. Omit it and the backend falls back to
`TASK_TOKEN_SECRET`, which couples the two: rotating `TASK_TOKEN_SECRET` would
then log every user out. `ADMIN_PASSWORD` (minimum 12 characters) seeds the
bootstrap admin; omit it and a random password is generated and logged once at
WARN on first boot.

## 4. Install

```bash
helm install moatless ./k8s/helm/moatless -n moatless -f k8s/local/values-local.yaml
```

## 5. Verify boot

```bash
# The real boot check — a bad DATABASE_URL does NOT crash the pod (the backend
# logs and continues, /health still returns 200), so a green pod is not proof.
# On a cold start the backend retries the DB connection for ~30s to ride out a
# Postgres that is still initializing.
kubectl -n moatless logs deploy/moatless-backend | grep "Database migrations complete"

# App reachable through Traefik. Anonymous /auth/me is 401 — that is correct.
curl -H 'Host: moatless.localhost' localhost/api/v1/auth/me      # 401
```

Open `http://moatless.localhost`, sign in at `/login`, and you are in.

## 6. First run — credential, repo, task

- **Login needs no OAuth.** `auth.provider` is empty (the default), so the
  backend runs local username/password auth. An `admin` account is seeded at
  boot from `ADMIN_PASSWORD` in the `moatless-secrets` Secret; when that key is
  absent a random password is generated and logged **once** at WARN:

  ```bash
  kubectl -n moatless logs deploy/moatless-backend | grep "generated a bootstrap admin password"
  ```

  Sign in at `http://moatless.localhost/login`, then change the password under
  Settings → Password.
- **Add the agent credential in the UI:** Settings → Secrets →
  `CLAUDE_CODE_OAUTH_TOKEN`. It is read only from the encrypted DB (never from
  chart values), so it needs a working `SECRETS_ENCRYPTION_KEY`.
- **Create a repository without a provider.** The seeded "Moatable" repo sets
  `provider: Github`, which trips the GitHub-token preflight and errors the task
  before any pod is created — even for a public, anonymously clonable repo. Create
  one omitting `provider`:

  ```bash
  # Log in once and reuse the session cookie, exactly as the browser does.
  COOKIE=$(curl -si -H 'Host: moatless.localhost' -H 'Content-Type: application/json' \
    localhost/api/v1/auth/password/login \
    -d '{"username":"admin","password":"<your admin password>"}' \
    | grep -i '^set-cookie:' | sed 's/.*\(moatless_session=[^;]*\).*/\1/')

  curl -X POST -H 'Host: moatless.localhost' -H 'Content-Type: application/json' \
    -H "Cookie: $COOKIE" \
    localhost/api/v1/repositories \
    -d '{"name":"local-smoke","remoteUrl":"https://github.com/aorwall/moatable","defaultBranch":"main","kind":"remote"}'
  ```

  The clone URL field is `remoteUrl` (not `git_url`), and `kind` is required. Omit
  `provider` so the GitHub-token preflight is skipped. `moat repos create` cannot
  do this — its `--provider` flag defaults to `github` — and by default `moat`
  targets the hosted instance, not this local cluster.

- **Create the task** (starts the sandbox; add a message to also run the agent —
  which needs `CLAUDE_CODE_OAUTH_TOKEN` from the step above):

  ```bash
  RID=<repository id from the create response>
  curl -X POST -H 'Host: moatless.localhost' -H 'Content-Type: application/json' \
    -H "Cookie: $COOKIE" \
    localhost/api/v1/tasks -d "{\"name\":\"smoke\",\"repositoryId\":\"$RID\"}"

  kubectl -n moatless get pods    # moatless-ws-<task-id> reaches 1/1 Running
  curl -H 'Host: moatless.localhost' -H "Cookie: $COOKIE" \
    localhost/api/v1/tasks/<task-id>   # environmentStatus: ready
  ```

  A ready sandbox has cloned the repo into `/opt/moatless/workspace/<repo>` and
  connected back to the backend over gRPC.

## 7. Dev loop — rebuild and redeploy

```bash
task dev:backend
```

Cross-compiles the backend, bakes it into the thin runtime image
(`Dockerfile.backend-cross` — one `COPY` onto distroless), rolls the Deployment,
and asserts the running image is the one just built. No registry push: kubeadm
mode shares the local Docker image store.

Measured on Apple Silicon, one-line change in `backend/src/`:

| Change | Time |
|---|---|
| No source change | ~15s |
| One-line backend change | ~1m20s |

The build uses the `ci` cargo profile (inherits `release`, drops thin LTO),
which is what keeps the relink cheap — thin LTO costs ~20s and ~200s of CPU per
iteration. Pass `DEV_PROFILE=release` to build what actually ships; the first
run after switching profiles rebuilds every dependency, so expect one slow pass.

`KUBECONFIG` and the context are pinned to `docker-desktop` inside the task,
not inherited: direnv points `KUBECONFIG` at a kubeconfig whose current context
is a remote EKS cluster, so an unpinned `kubectl` would roll out to the wrong
cluster.

The digest assertion at the end is not ceremony — `pullPolicy: IfNotPresent`
with a re-used `:dev` tag fails *silently*, leaving the previous image running.
It compares against pods in the newest ReplicaSet only, since `rollout status`
returns before the previous revision's pods finish terminating.

## Why these settings (gotchas)

| Setting | Why |
|---|---|
| `image.pullPolicy` / `backend.image.pullPolicy: IfNotPresent`, tag `dev` | Chart defaults both to `Always` + `latest`, which forces a Docker Hub pull and silently ignores locally built images. |
| `kubernetes.imagePrepull.enabled: false` | Pre-pulls the full 5-8GB sandbox image with `pullPolicy: Always`. |
| `kubernetes.sandboxStorage.defaultPvc: true`, `defaultS3: false` | The S3 snapshot needs a real bucket a local install does not have, so the per-task PVC alone carries the workspace. Docker Desktop's built-in hostpath provisioner backs it, so workspaces survive pod replacement and retained close/reopen without any cloud storage. |
| `kubernetes.sandboxSysctls` — drop `tcp_keepalive_*` | Some kubelets reject them as unsafe → block sandbox pod admission. |
| `networkPolicy.sandbox.enabled: false` | Expects a namespace literally named `traefik`; locally Traefik lives in `moatless`. |
| `proxy.domain: preview.moatless.localhost`, `protocol: http`, `traefikEntryPoint: web` | macOS resolves arbitrary `*.localhost` → `127.0.0.1`. Previews sit under the app host so they share its registrable domain (see `auth.cookieDomain`). No dnsmasq, no nip.io. |
| `proxy.traefikIngressRoute.enabled: true` | Renders the app's Traefik `IngressRoute` CRDs (locally Traefik *is* the router). |
| `proxy.traefikMiddleware.enabled: true`, `noAuth: false` | The backend stamps the `preview-auth` middleware onto every per-sandbox preview `IngressRoute`; without the CRD Traefik invalidates those routers and 404s every preview host. As real ForwardAuth it sends each preview request to `/api/v1/auth/verify`, which enforces the session and per-task access. Works over http because previews share the app's registrable domain, so the `SameSite=Lax` session cookie is sent to them. |
| `auth.cookieDomain: moatless.localhost` | Scopes the session cookie to the app host **and** every preview subdomain (`*.preview.moatless.localhost`), so preview ForwardAuth receives it. |
| `ingress.enabled: false`, `proxyIngress.enabled: false` | Both target a Service named `traefik:80` in-namespace — a cloud shape where an ALB fronts Traefik. Locally Traefik is the controller, so they are circular. |
| `ingress.hosts` **kept despite `enabled: false`** | Trap: the IngressRoute templates read the app host from `ingress.hosts[0].host` regardless of the enable flag. Omit it and all 7 routes fall back to `moatless.example.com` and match nothing. |
| `postgres.enabled: true` | Uses the chart's first-party Postgres template. |

## Notes

- **`task deploy` passes no values file** (`helm upgrade` with only
  `--set image.tag=...`), so it renders chart defaults. It is not the local path —
  use the `helm install ... -f k8s/local/values-local.yaml` above.
- **Docker Desktop Kubernetes can reset on restart** on some versions. If the
  cluster comes back empty, re-run steps 2-4 (the images are still in the store).
