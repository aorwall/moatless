# AWS EKS Fargate preset

## Intended use

This is a small shared or pilot Moatless installation for trusted repositories. It keeps operations simple by using EKS Fargate, S3 workspace synchronization, managed PostgreSQL, and an ALB in front of an in-cluster Traefik proxy.

Do not use this preset for repositories that may be hostile. EKS Fargate does not support EKS VPC CNI Kubernetes NetworkPolicy enforcement. Security groups isolate the backend from RDS, but they do not provide per-sandbox egress policy.

## Architecture

- One VPC across two availability zones.
- Public subnets for the ALB and one NAT gateway; private subnets for Fargate pods.
- Separate private database subnets.
- EKS 1.35 with private and restricted-public API endpoints.
- Fargate profiles for `kube-system`, the application namespace, and the sandbox namespace.
- RDS PostgreSQL 17 on `db.t4g.micro`, encrypted storage, one-day backups, AWS-managed master password, and a final snapshot on deletion.
- A versioned, encrypted S3 workspace bucket retained on destroy.
- AWS Load Balancer Controller chart 3.5.0 and Traefik chart 41.2.0.
- ACM certificate for the application hostname and wildcard preview hostname. Route53 can be managed directly; external DNS receives explicit certificate-validation and ALB-routing handoffs.

The ALB Ingress and Traefik Service must be in the application namespace because an Ingress backend cannot reference a Service in another namespace. Dynamic Traefik `IngressRoute` resources may live in the sandbox namespace.

## Runtime contract

The generated Moatless values use:

- `storage.backend: s3` and `storage.s3.createSharedVolume: false`;
- `kubernetes.computeMode: fargate`;
- S3 as the only sandbox workspace mode;
- no image-prepull DaemonSet, DinD, node archives, package hostPath caches, EBS workspace PVCs, or S3 CSI mount;
- non-root sandbox and server containers with no added Linux capabilities;
- the chart's existing resource limits promoted to equal requests and limits, which gives Fargate Guaranteed QoS;
- `40Gi` sandbox ephemeral-storage request/limit and a 900-second pod startup timeout;
- one replica for small-installation controllers and application components;
- T3 as the only web UI, with the unused Moatless frontend omitted;
- `latest` application images pulled on pod start for this pilot;
- sandbox NetworkPolicy disabled, with the limitation surfaced as a warning.

Repository overrides cannot re-enable DinD, node/EBS storage, node caches, or burstable CPU/memory in Fargate mode. The backend rejects such a provisioning plan before creating a pod.

## Prerequisites

- An AWS profile whose session is current and whose identity the user approves.
- Permissions for EKS, EC2/VPC, IAM, CloudFormation when using the CLI path, RDS, S3, ACM, Route53, Secrets Manager, and Elastic Load Balancing.
- A public DNS zone that contains both requested domains. Route53 is managed directly; external providers require manual CNAME records.
- AWS CLI v2, kubectl 1.31+, Python 3.11+, and a Helm release compatible with Kubernetes 1.35: Helm 3.20+ or Helm 4.1+.
- Terraform 1.14.x/1.15.x for the tested default path, or eksctl 0.229+ for the experimental CLI path.
- A restricted operator CIDR for the public EKS API endpoint. `0.0.0.0/0` and `::/0` are rejected.

## Cost and availability choices

The pilot preset uses a single NAT gateway, one database instance, and one replica for most in-cluster components. This reduces cost but does not provide multi-AZ data-plane failover. Fargate pod cost scales with requested CPU and memory; sandbox defaults are deliberately large enough for the existing Moatless limit contract.

Before apply, call out at least: EKS control plane, NAT gateway and processing, Fargate CPU/memory, RDS, ALB, S3, logs, and data transfer. Use the AWS Pricing Calculator for an estimate if the user needs a current number.

## Pinned infrastructure

- Terraform: `>= 1.14.0, < 1.16.0`
- AWS provider: `6.59.0`
- terraform-aws-vpc: `6.6.1`
- terraform-aws-eks: `21.24.2`
- terraform-aws-iam: `6.8.0`
- EKS: `1.35`
- AWS Load Balancer Controller chart: `3.5.0`
- Traefik chart: `41.2.0`

Treat a pin change as a new asset revision: research compatibility, validate from a clean render, and re-plan. Do not silently select latest versions.
