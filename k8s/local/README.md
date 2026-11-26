# Moatless on a local Kubernetes cluster

Run Moatless on a single-node Kubernetes cluster on your machine, using
locally built images and the chart's first-party Postgres. This is the
supported local path — no Docker Hub pull, no private registry mirror, no cloud.

`values-local.yaml` in this directory is the committed local config. It contains
**no secrets** — you create the Secret by hand (step 3 below).

## Prerequisites

- **Docker Desktop 4.74+ with Kubernetes enabled in kubeadm mode.** kubeadm mode
  shares the local Docker image store, so pods run locally built images with no
  registry push and no `kind load`. kind mode does not share the store.
- **Apple Silicon (arm64).**
- About **16GB memory** for the Docker Desktop VM and at least **40GB free host
  disk**. A full disk stops the Docker daemon in the middle of a build.
- A checkout of `soaplabs/t3code` beside this repository, or `T3CODE_DIR` set to
  its path.
- `kubectl`, `helm` and `jq`.

**Pass the context explicitly.** Your current context may be a remote cluster.
Every command below names `docker-desktop`.

## 1. Build the images (arm64, tagged `:dev`)

```bash
.agents/skills/k8s-install/scripts/build-local-images.sh            # all
.agents/skills/k8s-install/scripts/build-local-images.sh backend    # or one: backend, sandbox, t3
```

The script compiles Rust inside Linux containers, on the `rust-toolchain.toml`
version. `task build:backend` and `docker/build-sandbox.sh` cross-compile with
zig, which rejects the `--fix-cortex-a53-843419` linker argument of rustc 1.100
(seen with zig 0.15.2 and 0.16.0).

`docker image prune -a` deletes any of these images that no container uses.

## 2. Traefik + namespace

Traefik provides the `IngressRoute` CRDs the app routing (and the backend's
per-sandbox preview routes) depend on.

```bash
kubectl --context docker-desktop create namespace moatless
helm repo add traefik https://traefik.github.io/charts && helm repo update
helm --kube-context docker-desktop install traefik traefik/traefik -n moatless \
  --set service.type=LoadBalancer --set ports.web.exposedPort=80
```

## 3. Secrets (never in a committed values file)

```bash
kubectl --context docker-desktop -n moatless create secret generic moatless-secrets \
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
helm --kube-context docker-desktop install moatless ./k8s/helm/moatless -n moatless -f k8s/local/values-local.yaml
```

## 5. Verify boot

```bash
# A bad DATABASE_URL does not crash the pod and /health still returns 200, so
# check for the migration line.
kubectl --context docker-desktop -n moatless logs deploy/moatless-backend | grep "Database migrations complete"

# Anonymous /auth/me is 401, which is correct.
curl -H 'Host: moatless.localhost' localhost/api/v1/auth/me
```

Open `http://moatless.localhost` in a browser and sign in. The app must not stay
on "Still connecting": `curl` cannot show whether the UI connects.

## 6. First run

- **Sign in with the email `admin@example.com`** (`auth.local.adminEmail`), not
  `admin`. The password is `ADMIN_PASSWORD` from the Secret. Without that key a
  random password is logged once at WARN:

  ```bash
  kubectl --context docker-desktop -n moatless logs deploy/moatless-backend | grep "generated a bootstrap admin password"
  ```

- **Claude:** run `claude setup-token`, then save the token under Settings →
  Providers → Claude.
- **Private repositories:** save a GitHub token under Settings → Version control.
  A fine-grained token limited to the repositories you need, with read and write
  on Contents and Pull requests, is enough.
- **Add a repository with `"provider": "github"`.** It gets a workspace of the
  same name, and a task needs a workspace. A repository without a `provider`
  gets none. Save the GitHub token first: without it a task is refused, for a
  public repository too.

### Through the API

`moat` targets the hosted instance, so call the local API directly. Writes need
the session cookies, an `Origin` header, and `X-CSRF-Token` set to the
`moatless_csrf` cookie.

```bash
H=(-H 'Host: moatless.localhost' -H 'Origin: http://moatless.localhost' -H 'Content-Type: application/json')
read -rs PW   # the admin password
COOKIES=$(jq -n --arg p "$PW" '{username:"admin@example.com",password:$p}' \
  | curl -si "${H[@]}" localhost/api/v1/auth/password/login -d @- \
  | grep -i '^set-cookie:' | sed -E 's/^[^:]+: *([^;]+).*/\1/' | paste -sd';' -)
CSRF=$(echo "$COOKIES" | tr ';' '\n' | sed -n 's/^moatless_csrf=//p')
H+=(-H "Cookie: $COOKIES" -H "X-CSRF-Token: $CSRF")

curl "${H[@]}" localhost/api/v1/repositories \
  -d '{"name":"local-smoke","remoteUrl":"https://github.com/<owner>/<public-repo>","defaultBranch":"main","kind":"remote","provider":"github"}'
curl "${H[@]}" localhost/api/v1/workspaces        # note the new workspace id

curl "${H[@]}" localhost/api/v1/tasks \
  -d '{"name":"smoke","repositoryId":"<repository id>","workspaceId":"<workspace id>","agentProfile":"claude-code"}'

# The response carries `taskId`. The sandbox pod reaches 1/1 Running.
kubectl --context docker-desktop -n moatless get pods
```

A ready sandbox has cloned the repository into
`/opt/moatless/workspace/<repo>` and connected to the backend over gRPC.

## 7. Rebuild and redeploy

```bash
.agents/skills/k8s-install/scripts/build-local-images.sh backend
kubectl --context docker-desktop -n moatless rollout restart deploy/moatless-backend
kubectl --context docker-desktop -n moatless rollout status deploy/moatless-backend
```

For `t3`, restart `deploy/moatless-t3`. A new sandbox pod picks up a new
sandbox image. `pullPolicy: IfNotPresent` with a reused `:dev` tag keeps the old
image running until the restart.

After a chart or values change:

```bash
helm --kube-context docker-desktop upgrade moatless ./k8s/helm/moatless -n moatless -f k8s/local/values-local.yaml
```

## Upgrading an old release

Postgres here has no persistent volume: replacing its pod empties the database.
Dump it first:

```bash
kubectl --context docker-desktop -n moatless exec deploy/postgres -- pg_dump -U moatless moatless > moatless.sql
```

| Symptom | Fix |
|---|---|
| Migration fails with `every human user must have an email` | Set an email on each human row in `users`, or empty the database by deleting the Postgres pod, then restart the backend |
| Install fails because Service `postgres` belongs to release `moatless-vibe` | Dump the database, uninstall that release, and install `moatless`. Create `moatless-secrets` with the old `SECRETS_ENCRYPTION_KEY`, or restored secrets cannot be decrypted |
| Upgrade fails because Middleware `preview-auth` is not owned by Helm | Delete the Middleware and run the upgrade again |

## Why these settings (gotchas)

| Setting | Why |
|---|---|
| `backend.image`, `sandboxImage`, `t3.image`: `pullPolicy: IfNotPresent`, tag `dev` | The chart defaults to `latest`, mostly with `Always`, which forces a Docker Hub pull and silently ignores locally built images. |
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

- **`task dev:backend` builds with zig** and fails the same way as
  `task build:backend`; use step 7.
- **`task deploy` passes no values file** (`helm upgrade` with only
  `--set backend.image.tag=...`), so it renders chart defaults. It is not the local path —
  use the `helm install ... -f k8s/local/values-local.yaml` above.
- **Docker Desktop Kubernetes can reset on restart** on some versions. If the
  cluster comes back empty, re-run steps 2-4 (the images are still in the store).
