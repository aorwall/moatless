#!/bin/bash
# Deploy development images to Kubernetes
#
# Usage:
#   ./scripts/deploy-dev.sh                    # Build and deploy with auto-generated tag
#   ./scripts/deploy-dev.sh --tag my-feature   # Use custom tag
#   ./scripts/deploy-dev.sh --app-only         # Only deploy app (skip sandbox)
#   ./scripts/deploy-dev.sh --skip-build       # Skip build, just deploy (use existing tag)
#
# Prerequisites:
#   - Docker logged into Docker Hub
#   - kubectl configured for target cluster
#   - Helm installed

set -e

# Configuration
IMAGE_NAME="aorwall/moatless-frontend"
SANDBOX_IMAGE_NAME="aorwall/moatless-sandbox-js"
NAMESPACE="vibe"
HELM_RELEASE="moatless-vibe"
VALUES_FILE=""

# Parse arguments
TAG=""
SKIP_BUILD=false
APP_ONLY=false

while [[ $# -gt 0 ]]; do
  case $1 in
    --tag)
      TAG="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --app-only)
      APP_ONLY=true
      shift
      ;;
    *)
      echo "Unknown option: $1"
      exit 1
      ;;
  esac
done

# Generate tag if not provided
if [ -z "$TAG" ]; then
  SHORT_SHA=$(git rev-parse --short HEAD)
  TIMESTAMP=$(date +%Y%m%d-%H%M%S)
  TAG="dev-${SHORT_SHA}-${TIMESTAMP}"
fi

echo "=== Deploy Dev Image ==="
echo "Tag: $TAG"
echo "Namespace: $NAMESPACE"
echo ""

if [ "$SKIP_BUILD" = false ]; then
  # Step 1: Build the application package
  echo "==> Building application package..."
  bun run build:package

  # Step 2: Build and push Docker images
  echo ""
  echo "==> Building app image..."
  docker build \
    --platform linux/amd64 \
    -f docker/Dockerfile.app \
    -t "${IMAGE_NAME}:${TAG}" \
    .

  echo ""
  echo "==> Pushing app image..."
  docker push "${IMAGE_NAME}:${TAG}"

  if [ "$APP_ONLY" = false ]; then
    echo ""
    echo "==> Building sandbox image using build-sandbox.sh..."
    ./docker/build-sandbox.sh --tag "${TAG}" --only sandbox-js --skip-base --push
  fi
fi

# Step 3: Deploy with Helm
echo ""
echo "==> Deploying to Kubernetes..."

HELM_ARGS=(
  "${HELM_RELEASE}"
  "./k8s/helm/moatless-vibe"
  "-n" "${NAMESPACE}"
  "--set" "image.tag=${TAG}"
)
if [ -n "${VALUES_FILE}" ]; then
  HELM_ARGS+=("-f" "${VALUES_FILE}")
fi

if [ "$APP_ONLY" = false ]; then
  HELM_ARGS+=("--set" "kubernetes.sandboxImage.repository=${SANDBOX_IMAGE_NAME}")
  HELM_ARGS+=("--set" "kubernetes.sandboxImage.tag=${TAG}")
fi

helm upgrade "${HELM_ARGS[@]}"

# Step 4: Wait for rollout
echo ""
echo "==> Waiting for rollout..."
kubectl rollout status deployment/${HELM_RELEASE} -n ${NAMESPACE} --timeout=120s || true

# Step 5: Verify
echo ""
echo "==> Verifying deployment..."
kubectl get pods -n ${NAMESPACE} -l app.kubernetes.io/name=moatless-vibe

echo ""
echo "=== Deployment Complete ==="
echo "Image: ${IMAGE_NAME}:${TAG}"
if [ "$APP_ONLY" = false ]; then
  echo "Sandbox: ${SANDBOX_IMAGE_NAME}:${TAG}"
fi
echo ""
echo "Check logs: kubectl logs deployment/${HELM_RELEASE} -n ${NAMESPACE} --tail=50"
echo "Check health: curl -s https://moatless.xyz/ -o /dev/null -w '%{http_code}'"
