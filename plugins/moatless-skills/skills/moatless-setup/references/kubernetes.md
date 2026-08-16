# Kubernetes setup

Use this workflow for an existing shared or production cluster. It requires `kubectl`, Helm 3, a target namespace, the main application domain, and a separate or wildcard preview domain.

## Prepare

1. Confirm the active cluster before writing anything:

   ```bash
   kubectl config current-context
   moat k8s preflight --namespace <namespace> --json
   ```

   Continue only when the context is the intended cluster and every required check reports `ok: true`. Repair failures using each check's `hint`. Do not infer or switch a production context silently.

2. Determine the provider from preflight (`aks`, `eks`, or `generic`). Confirm the main app domain and wildcard preview domain, then generate the deployment values:

   ```bash
   moat k8s init \
     --provider <aks|eks|generic> \
     --domain <app.example.com> \
     --proxy-domain <preview.example.com> \
     --namespace <namespace> \
     --auth local
   ```

   `moat k8s init` does not yet accept `oidc`; `--auth local` creates a complete base file that the login reference updates before installation. If `values.local.yaml` already exists, inspect and preserve it. Do not pass `--force` without explicit authorization to overwrite that file.

3. Review `values.local.yaml` for the intended domains, ingress class, storage backend, image tag, namespace, and `secrets.create: false`. Keep credentials out of this file.

## Ready for login

Preparation is complete when preflight passes and `values.local.yaml` contains the intended non-auth deployment configuration. Return to `SKILL.md`, configure one login mode in that file and its required secret inputs, then resume below.

## Install and verify

1. Put the required credentials in an uncommitted `.env.local`, or have the user enter them through masked `moat k8s secrets set` prompts. Never ask the user to paste secrets into chat.

2. Bootstrap the application secret using the auth-configured values file:

   ```bash
   moat k8s secrets bootstrap \
     --namespace <namespace> \
     --values values.local.yaml
   moat k8s secrets list --namespace <namespace> --json
   ```

   Confirm `DATABASE_URL`, `SECRETS_ENCRYPTION_KEY`, and `TASK_TOKEN_SECRET` exist. For OIDC, also confirm `OIDC_CLIENT_ID` and `OIDC_CLIENT_SECRET`. For local login, confirm `ADMIN_PASSWORD` was set by the login reference.

3. Install the release and wait for rollout:

   ```bash
   moat k8s install \
     --namespace <namespace> \
     --release moatless-vibe \
     --values values.local.yaml \
     --wait
   moat k8s status \
     --namespace <namespace> \
     --release moatless-vibe \
     --json
   ```

   Require `all_healthy: true`. Inspect only named unhealthy deployments or pending pods with `kubectl -n <namespace> describe` and `kubectl -n <namespace> logs`, repair the cause, and re-run install and status.

4. Resolve the public URL from the configured app domain and reported ingress address. Verify:

   ```bash
   curl -fsS https://<app-domain>/api/v1/auth/mode
   ```

   Open `https://<app-domain>/login` for the real sign-in required by the main workflow.

The deployment checkpoint is complete when the release is healthy, ingress serves the configured auth mode, and persistent data remains intact.
