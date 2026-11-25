# Kubernetes setup

Use this workflow for an existing shared or production cluster. It requires `kubectl`, a Helm version compatible with the target Kubernetes release, a target namespace, the main application domain, and a separate or wildcard preview domain. The AWS EKS 1.35 handoff accepts Helm 3.20+ or Helm 4.1+. A new cloud cluster must first be provisioned by `$provision-moatless-kubernetes`; its `infrastructure-handoff.json` is the preferred input here.

## Prepare

1. Confirm the CLI supports context-pinned operations: `moat k8s status --help` must list `--context`. Rebuild or install the CLI from the matching checkout if it does not. When a handoff exists, use its exact context and Fargate/S3 contract without changing the user's current context:

   ```bash
   moat k8s preflight \
     --namespace <namespace> \
     --context <handoff-context> \
     --compute-mode fargate \
     --storage-mode s3 \
     --json
   ```

   Continue only when the context is the intended cluster and every required check reports `ok: true`. Repair failures using each check's `hint`. Do not infer or switch a production context silently.

2. When a handoff exists, generate the deployment values directly from it. This validates the schema, context, recorded image tag, domains, S3 bucket, certificate, and backend AWS role:

   ```bash
   moat k8s init \
     --infrastructure-handoff infrastructure-handoff.json \
     --context <handoff-context> \
     --auth local
   ```

   Without a handoff, determine the provider from preflight (`aks`, `eks`, or `generic`). Confirm the main app domain and wildcard preview domain, then generate the deployment values:

   ```bash
   moat k8s init \
     --provider <aks|eks|generic> \
     --domain <app.example.com> \
     --proxy-domain <preview.example.com> \
     --namespace <namespace> \
     --context <target-context> \
     --auth local
   ```

   `moat k8s init` accepts only `--auth local`; it creates a complete base file that the login reference updates before installation. If `values.local.yaml` already exists, inspect and preserve it. Do not pass `--force` without explicit authorization to overwrite that file.

3. Review `values.local.yaml` for the intended domains, ingress class, storage backend, image tag, namespace, and `secrets.create: false`. Keep credentials out of this file.

## Ready for login

Preparation is complete when preflight passes and `values.local.yaml` contains the intended non-auth deployment configuration. Return to `SKILL.md`, configure one login mode in that file and its required secret inputs, then resume below.

## Install and verify

1. Put the required credentials in an uncommitted `.env.local`, or have the user enter them through masked `moat k8s secrets set` prompts. For the AWS handoff, merge the locally generated `.env.infrastructure` into the uncommitted input or pass it directly with `--env-file`; never print it. Never ask the user to paste secrets into chat.

2. Bootstrap the application secret using the auth-configured values file:

   ```bash
   moat k8s secrets bootstrap \
     --context <handoff-context> \
     --namespace <namespace> \
     --values values.local.yaml \
     --env-file <.env.local-or-.env.infrastructure>
   moat k8s secrets list \
     --context <handoff-context> \
     --namespace <namespace> \
     --json
   ```

   Confirm `DATABASE_URL`, `SECRETS_ENCRYPTION_KEY`, and `TASK_TOKEN_SECRET` exist. For OIDC, also confirm `OIDC_CLIENT_ID` and `OIDC_CLIENT_SECRET`. For local login, confirm `ADMIN_PASSWORD` was set by the login reference.

3. Install the release and wait for rollout:

   ```bash
   moat k8s install \
     --context <handoff-context> \
     --namespace <namespace> \
     --release moatless \
     --values values.local.yaml \
     --wait
   moat k8s status \
     --context <handoff-context> \
     --namespace <namespace> \
     --release moatless \
     --json
   ```

   Require `release_status: "deployed"`, at least one reported deployment, and `all_healthy: true`. Inspect only named unhealthy deployments or pending pods with `kubectl --context <handoff-context> -n <namespace> describe` and `kubectl --context <handoff-context> -n <namespace> logs`, repair the cause, and re-run install and status.

4. Resolve the public URL from the configured app domain and reported ingress address. Verify:

   ```bash
   curl -fsS https://<app-domain>/api/v1/auth/mode
   ```

   Open `https://<app-domain>/login` for the real sign-in required by the main workflow.

   For a cluster created by `$provision-moatless-kubernetes`, show the two DNS names, obtain its separate DNS approval, and run `<provision-skill-dir>/scripts/provision.py finalize-dns --config setup.toml --approve DNS-<deployment>` after both ingresses report the same ALB hostname. Then perform this public check.

The deployment checkpoint is complete when the release is healthy, ingress serves the configured auth mode, and persistent data remains intact.
