# EKS Provider Notes

## Detection

`moat k8s preflight` reports `provider: eks` when node `/spec/providerID` starts with `aws://`.

## Ingress — AWS Load Balancer Controller

Install once per cluster:

```bash
# Add IAM OIDC provider
eksctl utils associate-iam-oidc-provider --cluster <name> --approve

# Create IAM policy + service account
eksctl create iamserviceaccount \
  --cluster=<name> \
  --namespace=kube-system \
  --name=aws-load-balancer-controller \
  --attach-policy-arn=arn:aws:iam::<account>:policy/AWSLoadBalancerControllerIAMPolicy \
  --override-existing-serviceaccounts --approve

# Install via helm
helm repo add eks https://aws.github.io/eks-charts
helm install aws-load-balancer-controller eks/aws-load-balancer-controller \
  -n kube-system --set clusterName=<name> \
  --set serviceAccount.create=false --set serviceAccount.name=aws-load-balancer-controller
```

Then set in `values.local.yaml`:

```yaml
ingress:
  className: alb
  annotations:
    alb.ingress.kubernetes.io/scheme: internet-facing
    alb.ingress.kubernetes.io/target-type: ip
    alb.ingress.kubernetes.io/listen-ports: '[{"HTTP":80},{"HTTPS":443}]'
    alb.ingress.kubernetes.io/ssl-redirect: "443"
    alb.ingress.kubernetes.io/certificate-arn: arn:aws:acm:<region>:<acct>:certificate/<id>
proxyIngress:
  className: alb
  annotations: { <same> }
```

`moat k8s init --provider eks` writes the first six automatically; append the ACM cert ARN afterward.

## DNS

Set a Route53 CNAME (or A-alias) to the ALB hostname after install:

```bash
kubectl -n <ns> get ingress moatless-vibe -o jsonpath='{.status.loadBalancer.ingress[0].hostname}'
```

Use the same ALB for `<domain>` and `*.<proxy-domain>` — AWS ALBs accept multiple host-based rules, and the chart will emit two Ingress resources that share one ALB if `alb.ingress.kubernetes.io/group.name` is set equally on both.

## Storage

EKS doesn't ship with a default StorageClass. Install the EBS CSI driver:

```bash
eksctl create addon --name aws-ebs-csi-driver --cluster <name> --force
```

Create a default class:

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: gp3
  annotations:
    storageclass.kubernetes.io/is-default-class: "true"
provisioner: ebs.csi.aws.com
parameters:
  type: gp3
  encrypted: "true"
volumeBindingMode: WaitForFirstConsumer
```

## TLS

ALBs terminate TLS natively — use AWS Certificate Manager (ACM):

```bash
aws acm request-certificate --domain-name "*.example.com" \
  --subject-alternative-names "example.com" --validation-method DNS
```

Set the ARN in `alb.ingress.kubernetes.io/certificate-arn` on both ingresses. Keep `ingress.tls: []` in values — the ALB does TLS, not the cluster.

## IAM for S3 (optional)

If using S3 for file storage, create an IRSA role and set `serviceAccount.annotations` in values:

```yaml
serviceAccount:
  create: true
  annotations:
    eks.amazonaws.com/role-arn: arn:aws:iam::<acct>:role/moatless-vibe-s3
```

## Common EKS issues

| Symptom | Cause | Fix |
|---|---|---|
| Ingress stays `<pending>` | ALB controller not installed, or SG rules block | `kubectl -n kube-system logs deploy/aws-load-balancer-controller` |
| PVC `Pending` + no events | EBS CSI driver not installed | `eksctl create addon --name aws-ebs-csi-driver --force` |
| `ImagePullBackOff` | ghcr-secret missing | `moat k8s secrets pull-token` |
| 502 after install | Target group health check on `/` — backend health is at `/health` | Add `alb.ingress.kubernetes.io/healthcheck-path: /health` to ingress annotations |
