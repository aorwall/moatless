---
name: k8s-install
description: "Install Moatless on Kubernetes using moat k8s. Use for a fresh Helm installation, AKS or EKS prerequisites, values generation, secrets, and deployment verification."
---

# Kubernetes installation

Use an explicit context and namespace throughout. Read
[prerequisites](references/prerequisites.md), then the applicable
[AKS](references/provider-aks.md) or [EKS](references/provider-eks.md) reference.
The [CLI parser](../../../crates/cli/src/commands/k8s/mod.rs) owns flags and defaults.

1. Run `moat k8s preflight --context <context> --namespace <namespace> --json`.
   Resolve failed checks using [preflight checks](references/preflight-checks.md).
   For Fargate, pass the matching `--compute-mode` and `--storage-mode` and use
   the infrastructure handoff; do not apply an EC2 storage recipe.
2. Generate a values file:

   ```bash
   moat k8s init --context <context> --provider <aks|eks|generic> \
     --namespace <namespace> --sandbox-namespace <sandbox-namespace> \
     --domain <app-domain> --proxy-domain <preview-domain> \
     --tag <release-tag> --auth <local|github_app|auth0> --output values.local.yaml
   ```

   Supply the requested values for automation; omitted choices may prompt.
   An existing output file requires `--force` to overwrite. For provisioned
   Fargate infrastructure, use `--infrastructure-handoff <file>` and retain its
   context, namespace, and compute/storage constraints.
3. Review [values](references/values-schema.md), including every image tag,
   storage, data stores, auth URLs, and both ingress paths. Render locally before
   applying. The provider examples require real domains, TLS, and infrastructure.
4. Bootstrap [secrets](references/secrets.md) from a local file or masked CLI
   prompts, then inspect key names:

   ```bash
   moat k8s secrets bootstrap --context <context> --namespace <namespace> \
     --values values.local.yaml --env-file .env.local
   moat k8s secrets list --context <context> --namespace <namespace> --json
   ```

5. Install and verify:

   ```bash
   moat k8s install --context <context> --namespace <namespace> \
     --values values.local.yaml --wait
   moat k8s status --context <context> --namespace <namespace> --json
   ```

   Verify application login and [manual smoke behavior](../e2e-testing/references/manual-smoke.md).
   Installation secrets do not include a mandatory fleet-wide Anthropic key;
   users provide agent credentials at runtime.

For rollout failures use [troubleshooting](references/troubleshooting.md).
For ongoing operations use [sandbox-troubleshooting](../sandbox-troubleshooting/SKILL.md).
