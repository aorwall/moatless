# Values review

The [chart defaults](../../../../k8s/helm/moatless-vibe/values.yaml) and templates
own the schema. `moat k8s init` generates the starting file; review it before apply.

| Setting | Check |
|---|---|
| `image.tag`, `backend.image.tag`, `sandboxImage.tag`, `t3.image.tag` | Use the same selected release; do not upgrade only two images. |
| `t3.enabled`, `t3.host` | T3 enabled with an empty host owns the app hostname. Disable T3 for the Moatless frontend, or assign a separate T3 host for both. |
| `ingress` | Installed class, correct hosts, TLS termination, provider annotations. |
| `proxyIngress` | Wildcard host and target Traefik service in the release namespace. |
| `proxy.domain` | Matches the preview DNS suffix. |
| `postgres.enabled` | False requires an external `DATABASE_URL`; true uses the bundled Postgres. |
| `externalSecrets.enabled`, `externalSecrets.secretName` | Reference the prepared app Secret. |
| `kubernetes.sandboxStorage` | Match the compute substrate, StorageClass, and namespace. |

Use [secret keys](secrets.md) for credentials; keep values files free of real keys.
The installer storage-mode option and workspace `ResourceConfig` are different
interfaces. Read the workspace schema for `pvcEnabled`/`s3Enabled` overrides.

Render a provider example without applying it:

```bash
helm template skill-check k8s/helm/moatless-vibe \
  --namespace <namespace> -f .agents/skills/k8s-install/examples/values-eks-minimal.yaml
```

Rendering proves template compatibility only. It does not prove DNS, certificates,
controller support, storage provisioning, or credentials.
