# Kubernetes Deployment with Helm

This directory contains the Helm chart for deploying Moatless to Kubernetes.

## Prerequisites

- Kubernetes cluster (Azure AKS, GKE, EKS, or local)
- `kubectl` configured with cluster credentials
- `helm` v3.x installed
- cert-manager installed (for TLS certificates)

## Quick Start

### 1. Create Namespace and Secrets

```bash
# Create namespace
kubectl create namespace moatless

# Create secrets with your API keys
kubectl create secret generic moatless-secrets \
  --namespace moatless \
  --from-literal=CLAUDE_CODE_OAUTH_TOKEN=<your-token> \
  --from-literal=ANTHROPIC_API_KEY=<your-key> \
  --from-literal=GITHUB_TOKEN=<your-token> \
  --from-literal=WORKER_API_KEY=$(openssl rand -hex 32)
```

### 2. Install the Helm Chart

```bash
# From the repository root
helm install moatless ./k8s/helm/moatless \
  --namespace moatless \
  --set externalSecrets.enabled=true
```

### 3. Upgrade the Chart

```bash
helm upgrade moatless ./k8s/helm/moatless --namespace moatless
```

## Configuration

See `helm/moatless/values.yaml` for all available configuration options.

### Key Configuration Options

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.className` | Ingress controller class | `webapprouting.kubernetes.azure.com` |
| `ingress.hosts[0].host` | Main API hostname | `moatless.xyz` |
| `proxyIngress.host` | Preview proxy wildcard hostname | `*.proxy.moatless.xyz` |
| `kubernetes.enabled` | Enable Kubernetes execution mode for workers | `true` |
| `persistence.enabled` | Enable persistent storage | `false` |

### Example Custom Values

```yaml
# custom-values.yaml
ingress:
  hosts:
    - host: moatless.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: my-moatless-tls
      hosts:
        - moatless.example.com

proxyIngress:
  host: "*.preview.example.com"
  tls:
    secretName: preview-wildcard-tls
    hosts:
      - "*.preview.example.com"

proxy:
  domain: "preview.example.com"
```

```bash
helm install moatless ./k8s/helm/moatless \
  --namespace moatless \
  -f custom-values.yaml
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Ingress Controller                       │
├─────────────────────────────┬───────────────────────────────────┤
│   moatless.xyz          │   *.proxy.moatless.xyz             │
│   (Main API/UI)             │   (Preview proxy)                 │
└──────────────┬──────────────┴──────────────┬────────────────────┘
               │                              │
               ▼                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                     moatless Service                             │
│              Port 8000 (api)  │  Port 8080 (proxy)               │
└──────────────────────────────────────────────────────────────────┘
               │                              │
               ▼                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                           API Pod                                 │
│  ┌─────────────────────┐    ┌─────────────────────────────────┐  │
│  │   Main API Server   │    │  Hostname Proxy Service         │  │
│  │   (port 8000)       │    │  (port 8080)                    │  │
│  │   - REST API        │    │  Routes *.proxy.moatless.xyz     │  │
│  │   - WebSocket       │    │  to worker pod preview ports    │  │
│  │   - Static UI       │    │                                 │  │
│  └─────────────────────┘    └─────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
               │
               │ Creates worker pods
               ▼
┌──────────────────────────────────────────────────────────────────┐
│                        Worker Pods                                │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  worker-task-xxx                                            │  │
│  │  - Clones git repository                                    │  │
│  │  - Runs Claude Code agent                                   │  │
│  │  - Serves preview on internal port                          │  │
│  │  - Reports events back to main API                          │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

## Monitoring

```bash
# Watch pods
kubectl get pods -n moatless -w

# Check pod logs
kubectl logs -n moatless -l app.kubernetes.io/name=moatless

# Check ingress status
kubectl get ingress -n moatless

# Check certificates
kubectl get certificates -n moatless
```

## Troubleshooting

### Pod Not Starting

```bash
# Check pod events
kubectl describe pod -n moatless <pod-name>

# Check for resource constraints
kubectl top nodes
```

### TLS Certificate Issues

```bash
# Check certificate status
kubectl describe certificate -n moatless moatless-tls

# Check cert-manager logs
kubectl logs -n cert-manager -l app=cert-manager
```

### Ingress Not Working

```bash
# Check ingress controller logs
kubectl logs -n app-routing-system -l app=nginx
```
