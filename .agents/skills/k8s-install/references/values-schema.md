# Values review

The [chart defaults](../../../../k8s/helm/moatless/values.yaml) and templates
own the schema. `moat k8s init` generates the starting file; review it before apply.

| Setting | Check |
|---|---|
| `backend.image.tag`, `sandboxImage.tag` | Use `latest` or pin both to the same Moatless release tag, such as `moat-cli-v0.1.1`. Docker Hub release images support Linux AMD64 and ARM64. |
| `t3.image.repository`, `t3.image.tag` | Use `aorwall/moatless-t3:latest` for Linux AMD64 or ARM64. T3 publishes separately, so verify any alternate tag independently. |
| `t3.enabled`, `t3.host` | Keep T3 enabled with an empty host, so T3 Code serves the app hostname. |
| `ingress` | Installed class, correct hosts, TLS termination, provider annotations. |
| `proxyIngress` | Wildcard host and target Traefik service in the release namespace. |
| `proxy.domain` | Matches the preview DNS suffix. |
| `postgres.enabled` | False requires an external `DATABASE_URL`; true uses the bundled Postgres. |
| `externalSecrets.enabled`, `externalSecrets.secretName` | Reference the prepared app Secret. |
| `kubernetes.sandboxStorage` | Match the compute substrate, StorageClass, and namespace. |
| `workspaceSync.githubOrgs`, `workspaceSync.orgSyncUserId` | Set both or neither; the user must hold a GitHub credential that reads every listed organization's private repositories. |
| `workspaceSync.identityGrants` | One entry per repository and Workspace key that may set `sandboxServiceAccount`, `networkProfile` or `nodeSelector`; each value must exist in the cluster. |
| `sandboxAccess.roleDefaults` | `admin` and `user` each name an enabled `sandboxAccess.profiles` key, or stay empty. A workspace with an identity grant keeps its own. |
| `workspaceSync.sandboxTokenScope` | `log` (default) or `enforce`. Enforce refuses a sandbox token's call outside its Sandbox and caps its role unless the Workspace's grant sets `agentAdmin: true`. |

Use [secret keys](secrets.md) for credentials; keep values files free of real keys.
The installer storage-mode option and workspace `ResourceConfig` are different
interfaces. Read the workspace schema for `pvcEnabled`/`s3Enabled` overrides.

Render a provider example without applying it:

```bash
helm template skill-check k8s/helm/moatless \
  --namespace <namespace> -f .agents/skills/k8s-install/examples/values-eks-minimal.yaml
```

Rendering proves template compatibility only. It does not prove DNS, certificates,
controller support, storage provisioning, or credentials.
