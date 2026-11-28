# Moatless on a local Kubernetes cluster

Run Moatless on a single-node Kubernetes cluster with the published
`aorwall` images and the chart's bundled Postgres. The backend, sandbox, and
T3 images support Linux AMD64 and ARM64.

`values-local.yaml` in this directory is the committed local config. It contains
**no secrets** — you create the Secret by hand (step 3 below).

## Prerequisites

Allow about **16 GB of memory** for the cluster and **40 GB of free disk**
for images and workspaces. Install `kubectl`, `helm`, and `jq`. The cluster
needs access to Docker Hub.

| | macOS (Apple Silicon) | Linux (x86_64 or arm64) |
|---|---|---|
| Cluster | Docker Desktop 4.74+, Kubernetes in **kubeadm** mode | [k3d](https://k3d.io) 5.9+ on Docker Engine (rootful) |
| Context | `docker-desktop` | `k3d-moatless` |
| Install | Docker Desktop settings | Arch: `pacman -S kubectl helm jq` and AUR `rancher-k3d-bin`; elsewhere the [k3d install script](https://k3d.io/stable/#installation) |

**Pass the context explicitly.** Your current context may be a remote cluster.
Every command below names `$CTX`:

```bash
CTX=docker-desktop   # macOS
CTX=k3d-moatless     # Linux
```

## 0. Create the cluster (Linux)

On macOS, enabling Kubernetes in Docker Desktop is this step.

```bash
k3d cluster create moatless -p "80:80@loadbalancer" \
  --k3s-arg "--disable=traefik@server:*" --kubeconfig-switch-context=false
```

The last flag keeps your current context as it is; k3d otherwise switches it
to `k3d-moatless`.

k3s bundles its own Traefik, which would clash with the one step 2 installs.
Its `servicelb` publishes that Traefik's LoadBalancer on host port 80, and its
`local-path` StorageClass backs the sandbox PVCs. kind has neither a
LoadBalancer nor a host-port path without extra setup, which is why this path
uses k3d.

Check that `*.localhost` resolves to loopback for CLI tools:
`getent hosts x.preview.moatless.localhost`. Browsers resolve it themselves.
If it prints nothing, add `myhostname` to the `hosts:` line of
`/etc/nsswitch.conf`. If kubelet logs `too many open files`, raise
`fs.inotify.max_user_instances` to 512 and `fs.inotify.max_user_watches` to
524288 under `/etc/sysctl.d/`.

## 1. Use the Docker Hub images

The local values inherit the chart's `aorwall/moatless-backend:latest`,
`aorwall/moatless-sandbox:latest`, and `aorwall/moatless-t3:latest` images.
Kubernetes pulls the backend and T3 images during install, and the sandbox
image when a task starts. The published T3 image allows HTTP preview frames at
`*.preview.moatless.localhost`.

To pin a Moatless release, set both `backend.image.tag` and
`sandboxImage.tag` to the same release tag, such as `moat-cli-v0.1.1`.
T3 publishes separately and can stay on `:latest`.

## 2. Traefik + namespace

Traefik provides the `IngressRoute` CRDs the app routing (and the backend's
per-sandbox preview routes) depend on.

```bash
kubectl --context "$CTX" create namespace moatless
helm repo add traefik https://traefik.github.io/charts && helm repo update
helm --kube-context "$CTX" install traefik traefik/traefik --version 43.0.0 -n moatless \
  --set service.spec.type=LoadBalancer --set ports.web.exposedPort=80
```

The chart's values schema rejects unknown keys, and 43.0.0 moved the service
type to `service.spec.type`. Check the key before raising the pin.

## 3. Secrets (never in a committed values file)

```bash
kubectl --context "$CTX" -n moatless create secret generic moatless-secrets \
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
helm --kube-context "$CTX" install moatless ./k8s/helm/moatless -n moatless -f k8s/local/values-local.yaml
```

## 5. Verify boot

```bash
# A bad DATABASE_URL does not crash the pod and /health still returns 200, so
# check for the migration line.
kubectl --context "$CTX" -n moatless logs deploy/moatless-backend | grep "Database migrations complete"

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
  kubectl --context "$CTX" -n moatless logs deploy/moatless-backend | grep "generated a bootstrap admin password"
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
- **To try a public repository without a GitHub token,** add it with
  `"kind": "template"`. Preflight skips the credential check and the sandbox
  clones anonymously; pushing is still refused.
- **A preview needs a Script with a `port`.** A Workspace takes its Scripts
  from the repository's `.moatless/workspaces.json`; a repository without one
  can declare a Script on a single task (below).

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
  -d '{"name":"smoke","repositoryId":"<repository id>","workspaceId":"<workspace id>","agentProfile":"claude-code","message":"Change the main heading. Do not commit."}'

# The response carries `taskId`. The sandbox pod reaches 1/1 Running.
kubectl --context "$CTX" -n moatless get pods

# Declare and start a Script on that task alone. The response's `url` is the
# preview; open it in the signed-in browser, since anonymous requests get 401.
curl "${H[@]}" localhost/api/orchestration/scripts/<task id>/preview/run \
  -d '{"command":"bun install && PORT=5173 bun run dev","port":5173}'
```

A ready sandbox has cloned the repository into
`/opt/moatless/workspace/<repo>` and connected to the backend over gRPC.

## 7. Refresh released images or use local builds

For `:latest`, restart the backend and T3 Deployments to pull the current
images. New sandbox pods pull the current sandbox image when they start.
For a pinned release, change both tags and run the Helm upgrade with the
updated values.

```bash
kubectl --context "$CTX" -n moatless rollout restart deploy/moatless-backend deploy/moatless-t3
kubectl --context "$CTX" -n moatless rollout status deploy/moatless-backend
kubectl --context "$CTX" -n moatless rollout status deploy/moatless-t3
```

To test a local change, pass `backend`, `sandbox`, or `t3` to
`.agents/skills/k8s-install/scripts/build-local-images.sh`. On Linux, set
`K3D_CLUSTER=moatless` so the script imports the image into k3d. For a local
T3 build, set `T3CODE_DIR` if the checkout of `soaplabs/t3code` is outside
the default sibling directory. The backend and sandbox build in Linux
containers using the workspace's Rust version.

Select `:dev` and `IfNotPresent` for the image you built. For a local backend:

```bash
helm --kube-context "$CTX" upgrade moatless ./k8s/helm/moatless \
  -n moatless -f k8s/local/values-local.yaml \
  --set backend.image.tag=dev --set backend.image.pullPolicy=IfNotPresent
```

For a local sandbox or T3, use `sandboxImage.tag=dev` and
`sandboxImage.pullPolicy=IfNotPresent`, or `t3.image.tag=dev` and
`t3.image.pullPolicy=IfNotPresent`. Keep the corresponding overrides on
subsequent upgrades while using local images. After rebuilding a reused `:dev` tag,
restart the backend or T3 Deployment, or create a new sandbox pod.

## Upgrading an old release

Postgres here has no persistent volume: replacing its pod empties the database.
Dump it first:

```bash
kubectl --context "$CTX" -n moatless exec deploy/postgres -- pg_dump -U moatless moatless > moatless.sql
```

| Symptom | Fix |
|---|---|
| Migration fails with `every human user must have an email` | Set an email on each human row in `users`, or empty the database by deleting the Postgres pod, then restart the backend |
| Install fails because Service `postgres` belongs to release `moatless-vibe` | Dump the database, uninstall that release, and install `moatless`. Create `moatless-secrets` with the old `SECRETS_ENCRYPTION_KEY`, or restored secrets cannot be decrypted |
| Upgrade fails because Middleware `preview-auth` is not owned by Helm | Delete the Middleware and run the upgrade again |

## Why these settings (gotchas)

| Setting | Why |
|---|---|
| `kubernetes.imagePrepull.enabled: false` | Pre-pulls the full 5-8GB sandbox image with `pullPolicy: Always`. |
| `kubernetes.sandboxStorage.defaultPvc: true`, `defaultS3: false` | The S3 snapshot needs a real bucket a local install does not have, so the per-task PVC alone carries the workspace. The cluster's default StorageClass backs it (Docker Desktop's hostpath, k3s's `local-path`), so workspaces survive pod replacement and retained close/reopen without any cloud storage. |
| `kubernetes.sandboxSysctls` — drop `tcp_keepalive_*` | Some kubelets reject them as unsafe → block sandbox pod admission. |
| `networkPolicy.sandbox.enabled: false` | Expects a namespace literally named `traefik`; locally Traefik lives in `moatless`. |
| `proxy.domain: preview.moatless.localhost`, `protocol: http`, `traefikEntryPoint: web` | Browsers, macOS, and Linux with systemd-resolved or `nss-myhostname` resolve any `*.localhost` → `127.0.0.1`. Previews sit under the app host so they share its registrable domain (see `auth.cookieDomain`). No dnsmasq, no nip.io. |
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
  cluster comes back empty, re-run steps 2–4.
- **`k3d cluster delete moatless`** removes the cluster and its imported
  images; the host's Docker images stay. Recreate it from step 0 and re-import.
