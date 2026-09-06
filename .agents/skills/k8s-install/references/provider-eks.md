# EKS

The [EKS values example](../examples/values-eks-minimal.yaml) targets standard EKS
with an ALB Ingress controller and EC2-backed PVC storage. Replace the certificate
ARN, domains, namespace-specific routing, and StorageClass with actual resources.

For a standard cluster, follow the current
[AWS Load Balancer Controller installation](https://docs.aws.amazon.com/eks/latest/userguide/lbc-helm.html),
including IAM/service-account prerequisites. EKS Auto Mode manages load balancing
differently; inspect the compute arrangement before installing duplicate controllers.
An ALB routes preview traffic to the Traefik service; Traefik and its CRDs still
serve sandbox routes. See [prerequisites](prerequisites.md).

For EBS-backed workspaces, follow the
[EBS CSI prerequisites](https://docs.aws.amazon.com/eks/latest/userguide/ebs-csi.html)
and verify a real StorageClass; the name `gp3` alone does not create one. EBS
volumes cannot mount to Fargate pods. Use the CLI's validated infrastructure handoff
and matching compute/storage mode for Fargate instead of this example.

```bash
kubectl --context <context> get ingressclass
kubectl --context <context> get storageclass
kubectl --context <context> -n <namespace> get ingress,services
```

Verify ACM coverage for both app and wildcard preview hosts and keep all app,
backend, sandbox, and T3 images on the selected release tag.
