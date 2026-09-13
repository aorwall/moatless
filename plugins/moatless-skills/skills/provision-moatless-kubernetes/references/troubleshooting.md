# Troubleshooting

Resume the failed phase. Do not delete retained data or create a second deployment to hide a partial failure.

## Preflight

- STS or permission error: run `aws sts get-caller-identity --profile <profile>` and refresh the intended SSO/profile session. Do not fall back to another principal silently.
- Unsupported Helm version: EKS 1.35 requires Helm 3.20+ or Helm 4.1+; select a compatible binary and rerun preflight.
- DNS mismatch: correct the domains and configured zone. For Route53, also verify the hosted zone ID; do not create records in an unrelated zone.
- Tool version mismatch: install the pinned range, then rerun preflight.

## Bootstrap or plan

- Existing bucket owned elsewhere: inspect its tags and ownership marker. Choose a new globally unique name unless it is proven to belong to this deployment.
- Config hash mismatch: rerun `render` and `plan`. Do not bypass the hash or identity check.
- Terraform initialization error: verify the state bucket, region, and profile in `backend.hcl`.
- Method mismatch: use a fresh work directory or return `provisioning_method` to the original value.
- Tagged state bucket without an ownership marker: rerun the approved bootstrap plan. Bootstrap repairs the marker before configuring the workspace bucket, so a partial first attempt is resumable.

## Apply

- CoreDNS pending: confirm the `kube-system` Fargate profile and that CoreDNS is configured for Fargate compute.
- Fargate pod pending: describe it and inspect resource combinations, namespace profile selection, subnet IP capacity, and Pod execution role events.
- AWS Load Balancer Controller failure: inspect its service-account role annotation and controller logs before changing IAM.
- Backend cannot reach RDS: confirm the backend `SecurityGroupPolicy` selected the backend label and contains both the EKS cluster security group and backend security group.

## Moatless install

- `moat k8s init` rejects the handoff: regenerate it with `verify`; never hand-edit away schema or context validation.
- Sandbox plan rejected: Fargate intentionally blocks `dockerAccess`, EBS/node workspace overrides, node caches, and unequal CPU/memory requests and limits.
- S3 workspace sync fails: verify backend IRSA, bucket region/name, and S3 object permissions. No Mountpoint CSI pod or PVC should exist in this preset.
- Preview route unavailable: Traefik must be installed in the application namespace and the proxy Ingress must target its ClusterIP Service there.
- Helm reports `pending-install` while pods are healthy: treat the install as incomplete. Preserve the external Secret and data stores, repair the release record deliberately, then rerun the same context-pinned install and require Helm status `deployed`.

## DNS and retirement

- External ACM certificate pending: add every CNAME in `dns-validation-records.json` at the authoritative DNS provider, leave proxying off for validation, then rerun `verify`.
- External application DNS: add the apex-flattened and wildcard CNAMEs from `dns-handoff.json`; the provisioning script does not possess or request provider credentials.
- `finalize-dns` finds zero ALBs: wait for both Moatless Ingress resources to report status and inspect controller events.
- It finds multiple hostnames: verify both ingresses share the generated ALB group annotation before changing DNS.
- Destroy stalls on load balancers: remove Ingress resources while the AWS controller still runs, then retry the destroy plan.
- RDS deletion fails because a final snapshot name exists: retain the existing snapshot, choose a unique final snapshot identifier in the rendered Terraform asset, re-plan, and retry. Never set `skip_final_snapshot = true` merely to unblock teardown.
- Bucket deletion fails during an approved purge: list object versions and delete markers. Purge authorization must explicitly cover both named buckets.
