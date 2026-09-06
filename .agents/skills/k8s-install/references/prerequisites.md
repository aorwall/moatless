# Installation prerequisites

Confirm kubectl access to the named context, a compatible Helm installation,
application and wildcard preview DNS, TLS termination, image access, database
connectivity, and the selected sandbox storage/compute arrangement.

The chart creates standard `Ingress` resources for the app and wildcard proxy.
The proxy Ingress forwards to `proxyIngress.targetServiceName` (default `traefik`)
in the release namespace. Sandbox provisioning creates Traefik `IngressRoute`
resources. An `alb` or Azure ingress class does not replace that Traefik hop.
Install/configure Traefik and its CRDs, service, namespace watching, and preview
auth middleware to match the chart and sandbox namespace.

Read [proxy-ingress.yaml](../../../../k8s/helm/moatless-vibe/templates/proxy-ingress.yaml)
and [sandbox ingress](../../../../crates/sandbox-k8s/src/ingress.rs) before changing
routing. A Gateway API controller alone does not consume the chart's Ingress
resources; this chart does not generate a Gateway/HTTPRoute migration.

Use [AKS](provider-aks.md) or [EKS](provider-eks.md) instructions for cloud-specific
constraints. Recheck upstream support at installation time rather than copying a
fixed controller/chart version into this skill.
