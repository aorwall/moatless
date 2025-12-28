# Kubernetes Deployment with Helm

This directory contains the Helm chart for deploying Moatless Vibe to Kubernetes.

## Prerequisites

- Kubernetes cluster (Azure AKS, GKE, EKS, or local)
- `kubectl` configured with cluster credentials
- `helm` v3.x installed
- cert-manager installed (for TLS certificates)

## Quick Start

### 1. Create Namespace and Secrets

```bash
# Create namespace
kubectl create namespace vibe

# Create secrets with your API keys
kubectl create secret generic vibe-secrets \
  --namespace vibe \
  --from-literal=CLAUDE_CODE_OAUTH_TOKEN=<your-token> \
  --from-literal=ANTHROPIC_API_KEY=<your-key> \
  --from-literal=GITHUB_TOKEN=<your-token> \
  --from-literal=WORKER_API_KEY=$(openssl rand -hex 32)
```

### 2. Install the Helm Chart

```bash
# From the repository root
helm install vibe ./k8s/helm/moatless-vibe \
  --namespace vibe \
  --set externalSecrets.enabled=true
```

### 3. Upgrade the Chart

```bash
helm upgrade vibe ./k8s/helm/moatless-vibe --namespace vibe
```

## Configuration

See `helm/moatless-vibe/values.yaml` for all available configuration options.

### Key Configuration Options

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.className` | Ingress controller class | `webapprouting.kubernetes.azure.com` |
| `ingress.hosts[0].host` | Main API hostname | `vibe.moatless.ai` |
| `proxyIngress.host` | Preview proxy wildcard hostname | `*.proxy.moatless.ai` |
| `proxy.enabled` | Enable hostname-based preview routing | `true` |
| `kubernetes.enabled` | Enable Kubernetes execution mode for workers | `true` |
| `persistence.enabled` | Enable persistent storage | `false` |

### Example Custom Values

```yaml
# custom-values.yaml
ingress:
  hosts:
    - host: my-vibe.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: my-vibe-tls
      hosts:
        - my-vibe.example.com

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
helm install vibe ./k8s/helm/moatless-vibe \
  --namespace vibe \
  -f custom-values.yaml
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Ingress Controller                       │
├─────────────────────────────┬───────────────────────────────────┤
│   vibe.moatless.ai          │   *.proxy.moatless.ai             │
│   (Main API/UI)             │   (Preview proxy)                 │
└──────────────┬──────────────┴──────────────┬────────────────────┘
               │                              │
               ▼                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                     vibe-moatless-vibe Service                    │
│              Port 8000 (api)  │  Port 8080 (proxy)               │
└──────────────────────────────────────────────────────────────────┘
               │                              │
               ▼                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                           API Pod                                 │
│  ┌─────────────────────┐    ┌─────────────────────────────────┐  │
│  │   Main API Server   │    │  Hostname Proxy Service         │  │
│  │   (port 8000)       │    │  (port 8080)                    │  │
│  │   - REST API        │    │  Routes *.proxy.moatless.ai     │  │
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
kubectl get pods -n vibe -w

# Check pod logs
kubectl logs -n vibe -l app.kubernetes.io/name=moatless-vibe

# Check ingress status
kubectl get ingress -n vibe

# Check certificates
kubectl get certificates -n vibe
```

## Troubleshooting

### Pod Not Starting

```bash
# Check pod events
kubectl describe pod -n vibe <pod-name>

# Check for resource constraints
kubectl top nodes
```

### TLS Certificate Issues

```bash
# Check certificate status
kubectl describe certificate -n vibe vibe-tls

# Check cert-manager logs
kubectl logs -n cert-manager -l app=cert-manager
```

### Ingress Not Working

```bash
# Check ingress controller logs
kubectl logs -n app-routing-system -l app=nginx
```
