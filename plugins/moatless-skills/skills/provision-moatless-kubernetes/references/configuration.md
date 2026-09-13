# Configuration and handoff

## `setup.toml`

`assets/setup.example.toml` is the schema example. Prefer `scripts/provision.py init` because it verifies STS, discovers two available zones, and derives account-scoped S3 bucket names.

Required invariants:

- `schema_version = 1`, `provider = "aws"`, and `compute_mode = "fargate"`.
- `provisioning_method` is exactly `terraform` or `cli`.
- `cluster_version = "1.35"`. `image_tag` defaults to `latest` for the pilot; use a release or `sha-*` tag when reproducible rollouts are required.
- `dns_provider` is `route53` or `external`. External DNS is supported by the Terraform path and writes record handoffs instead of mutating the provider.
- Two distinct availability zones.
- At least one restricted API CIDR.
- State and workspace bucket names are globally unique and different.
- Application and preview domains belong to `dns_zone`. `route53_zone_id` is required only for Route53 mode.

`tags` are applied where the underlying tool supports them. Do not put credentials or connection URLs in this file.

## Method ownership

Bootstrap writes `ownership/<deployment>.json` into the retained state bucket before configuring the workspace bucket, so a partial first attempt can resume. Render writes a local method marker, config hash, and source-asset hash. It recreates the generated method and Kubernetes directories so removed resources cannot survive from an older asset revision. A work directory cannot switch between Terraform and CLI, and a plan cannot be applied after configuration or source-asset changes or under another AWS principal.

Terraform uses `backend.hcl` and key `deployments/<deployment>/terraform.tfstate`. The experimental CLI path uses CloudFormation for the foundation and eksctl for EKS/Fargate and service-account identities. Both render the same Kubernetes add-ons and handoff schema, but only Terraform has completed a live create/install verification.

External DNS has two explicit handoffs. After infrastructure apply, add the ACM CNAMEs from `dns-validation-records.json`; `verify` refuses a pending certificate. After the application is healthy, `finalize-dns` writes `dns-handoff.json` with the application and wildcard preview CNAMEs. Route53 mode performs the final aliases after `DNS-<deployment>` approval.

## `infrastructure-handoff.json`

`verify` writes schema version 1 with these non-secret fields:

```json
{
  "schemaVersion": 1,
  "provider": "aws",
  "computeMode": "fargate",
  "context": "moatless-dev-eu-north-1",
  "appNamespace": "moatless",
  "sandboxNamespace": "moatless-sandboxes",
  "region": "eu-north-1",
  "appDomain": "moatless.example.com",
  "previewDomain": "preview.example.com",
  "certificateArn": "arn:aws:acm:...",
  "workspaceBucket": "...",
  "backendRoleArn": "arn:aws:iam::...:role/...",
  "databaseSecretArn": "arn:aws:secretsmanager:...",
  "databaseEndpoint": "...rds.amazonaws.com",
  "databaseName": "moatless",
  "databasePort": 5432,
  "provisioningMethod": "terraform",
  "imageTag": "latest"
}
```

The handoff contains identifiers and endpoints, not passwords. `moat k8s init --infrastructure-handoff` validates every field, checks the exact kubectl context, and generates the Fargate/S3 Helm preset with the recorded image tag.

`secrets-env` reads the RDS-managed password locally and writes only `DATABASE_URL` to a mode-`0600` file. Never print or commit the file.
