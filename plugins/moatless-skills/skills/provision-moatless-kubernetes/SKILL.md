---
name: provision-moatless-kubernetes
description: "Provision or retire cloud Kubernetes infrastructure for Moatless. Use when creating a new AWS EKS cluster, especially the S3-only EKS Fargate preset, or when the Moatless setup workflow has no existing cluster. Includes the tested Terraform path, an experimental eksctl/AWS CLI path, approval-gated changes, verification, and an infrastructure handoff. AKS, GKE, and EKS managed-node presets are not implemented yet."
---

# Provision Moatless Kubernetes

Create the cloud substrate and cluster add-ons, then hand a verified cluster to `$moatless-setup`. The implemented preset is AWS EKS Fargate with S3 workspaces, no image prepull, no DinD, and no node-local volumes.

## Workflow

1. Confirm that the requested target is `aws` + `fargate`. If the user asks for AKS, GKE, or EKS managed nodes, explain that the preset is not present; do not adapt the Fargate assets by guesswork.

2. Read `references/aws-eks-fargate.md` and `references/configuration.md` completely. State the Fargate trust boundary: EKS VPC CNI NetworkPolicy enforcement is unavailable, so this preset is for trusted repositories only.

3. Resolve `<skill-dir>` to the directory containing this `SKILL.md`. Run `<skill-dir>/scripts/provision.py`; shell paths are relative to the caller's working directory, not automatically to the skill. Create `setup.toml` with its `init` command, then run `preflight`. Preflight is read-only. Show the user the exact AWS account, principal ARN, region, domains, method, and cost drivers before requesting any approval.

4. Create `bootstrap-plan.json`. After explicit approval, execute bootstrap with the exact token printed below. Bootstrap creates only the versioned, encrypted state and workspace buckets and records the chosen provisioning method.

   ```bash
   python3 <skill-dir>/scripts/provision.py bootstrap-plan --config setup.toml
   python3 <skill-dir>/scripts/provision.py bootstrap --config setup.toml \
     --plan bootstrap-plan.json --approve BOOTSTRAP-<deployment>
   ```

5. Render and plan one method. Terraform is the tested default. `cli` is an experimental AWS CLI + CloudFormation + eksctl parity path until a clean create/destroy execution test is recorded. Never switch a work directory between methods.

   ```bash
   python3 <skill-dir>/scripts/provision.py render --config setup.toml
   python3 <skill-dir>/scripts/provision.py plan --config setup.toml
   ```

6. Present the plan and cost-bearing resources. Apply only after the user explicitly approves the shown account, region, domains, and costs:

   ```bash
   python3 <skill-dir>/scripts/provision.py apply --config setup.toml \
     --approve APPLY-<deployment>
   ```

   This creates EKS, Fargate profiles, private data services, controller identities, AWS Load Balancer Controller, Traefik, and the backend pod security-group policy. It does not install Moatless.

7. For external DNS, add the ACM validation CNAMEs from `dns-validation-records.json`. Then verify and emit the versioned handoff:

   ```bash
   python3 <skill-dir>/scripts/provision.py verify --config setup.toml
   python3 <skill-dir>/scripts/provision.py secrets-env --config setup.toml \
     --output .env.infrastructure
   ```

   The secrets command writes mode `0600` and never prints values. Add the file to local ignore rules. Then invoke `$moatless-setup` with `infrastructure-handoff.json`. After Helm reports healthy ingresses, run `finalize-dns --approve DNS-<deployment>`. Route53 mode upserts aliases; external mode writes `dns-handoff.json` without changing DNS.

8. For failures, read `references/troubleshooting.md`. Diagnose the failed phase and resume it; do not bootstrap a second environment.

## Destruction

Create and show a destroy plan first. `destroy` removes the application release if present, the cluster, network, cache, and database instance; it takes a final RDS snapshot and retains both S3 buckets.

```bash
python3 <skill-dir>/scripts/provision.py destroy-plan --config setup.toml
python3 <skill-dir>/scripts/provision.py destroy --config setup.toml \
  --approve DESTROY-<deployment>
```

Only a separate `PURGE-<deployment>` approval may permanently delete all versions from the retained state and workspace buckets. Never infer purge authorization from destroy authorization.

## Safety rules

- Never expose AWS credentials, database passwords, connection URLs, API keys, or secret file contents in chat or command output.
- Never execute `apply`, `bootstrap`, `destroy`, `purge`, or DNS mutation without its exact approval boundary.
- Never use the current kubectl context implicitly. The setup creates and checks a distinct `<deployment>-<region>` context.
- Record the requested image tag in the handoff. The pilot default is `latest`, so each pod start may pull a newer build; use a release or `sha-*` tag when reproducibility matters. Keep tool/module/chart pins, provisioning method, and handoff schema intact. Re-plan after any config or rendered-asset change.
- Do not claim production-grade sandbox isolation on this Fargate preset. Recommend EKS managed nodes with enforced NetworkPolicy when untrusted repositories are in scope.
