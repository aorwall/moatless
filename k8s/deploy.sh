#!/bin/bash
set -e

# Configuration
IMAGE_NAME="${IMAGE_NAME:-aorwall/moatless-vibe-worker}"
IMAGE_TAG="${IMAGE_TAG:-latest}"
NAMESPACE="${NAMESPACE:-vibe}"
RELEASE_NAME="${RELEASE_NAME:-vibe}"
HELM_CHART_PATH="./k8s/helm/moatless-vibe"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

print_step() {
    echo -e "${GREEN}==>${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}Warning:${NC} $1"
}

print_error() {
    echo -e "${RED}Error:${NC} $1"
}

usage() {
    cat << EOF
Usage: $0 [OPTIONS]

Build and deploy Moatless Vibe to Kubernetes.

Options:
    --build-only      Only build the Docker image, don't push or deploy
    --push-only       Only push the Docker image (assumes already built)
    --deploy-only     Only deploy with Helm (assumes image already pushed)
    --skip-build      Skip Docker build, only push and deploy
    --tag TAG         Docker image tag (default: latest)
    --image IMAGE     Docker image name (default: aorwall/moatless-vibe-worker)
    --namespace NS    Kubernetes namespace (default: vibe)
    --values FILE     Values file for Helm (e.g., ./k8s/helm/values-moatless.yaml)
    -h, --help        Show this help message

Examples:
    $0                                      # Full build, push, and deploy
    $0 --build-only                         # Only build locally
    $0 --deploy-only                        # Only upgrade Helm release
    $0 --tag v1.0.0                         # Build and deploy with specific tag
    $0 --values ./k8s/helm/values-moatless.yaml  # Deploy with custom values
EOF
    exit 0
}

# Parse arguments
BUILD=true
PUSH=true
DEPLOY=true
VALUES_FILE=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --build-only)
            PUSH=false
            DEPLOY=false
            shift
            ;;
        --push-only)
            BUILD=false
            DEPLOY=false
            shift
            ;;
        --deploy-only)
            BUILD=false
            PUSH=false
            shift
            ;;
        --skip-build)
            BUILD=false
            shift
            ;;
        --tag)
            IMAGE_TAG="$2"
            shift 2
            ;;
        --image)
            IMAGE_NAME="$2"
            shift 2
            ;;
        --namespace)
            NAMESPACE="$2"
            shift 2
            ;;
        --values)
            VALUES_FILE="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            print_error "Unknown option: $1"
            usage
            ;;
    esac
done

FULL_IMAGE="${IMAGE_NAME}:${IMAGE_TAG}"

# Change to repository root
cd "$(dirname "$0")/.."

# Step 1: Build
if [ "$BUILD" = true ]; then
    print_step "Building application..."
    bun run build

    print_step "Building Docker image: ${FULL_IMAGE}"
    docker build --platform linux/amd64 -t "${FULL_IMAGE}" -f docker/Dockerfile .
fi

# Step 2: Push
if [ "$PUSH" = true ]; then
    print_step "Pushing Docker image: ${FULL_IMAGE}"
    docker push "${FULL_IMAGE}"
fi

# Step 3: Deploy
if [ "$DEPLOY" = true ]; then
    print_step "Deploying to Kubernetes namespace: ${NAMESPACE}"

    # Check if namespace exists
    if ! kubectl get namespace "${NAMESPACE}" &>/dev/null; then
        print_warning "Namespace ${NAMESPACE} does not exist. Creating..."
        kubectl create namespace "${NAMESPACE}"
    fi

    # Build helm args
    HELM_ARGS=(
        --namespace "${NAMESPACE}"
        --set image.tag="${IMAGE_TAG}"
        --set externalSecrets.enabled=true
    )

    # Add values file if provided
    if [ -n "${VALUES_FILE}" ]; then
        if [ ! -f "${VALUES_FILE}" ]; then
            print_error "Values file not found: ${VALUES_FILE}"
            exit 1
        fi
        print_step "Using values file: ${VALUES_FILE}"
        HELM_ARGS+=(-f "${VALUES_FILE}")
    fi

    # Check if release exists
    if helm status "${RELEASE_NAME}" -n "${NAMESPACE}" &>/dev/null; then
        print_step "Upgrading existing Helm release: ${RELEASE_NAME}"
        helm upgrade "${RELEASE_NAME}" "${HELM_CHART_PATH}" "${HELM_ARGS[@]}"
    else
        print_step "Installing new Helm release: ${RELEASE_NAME}"
        helm install "${RELEASE_NAME}" "${HELM_CHART_PATH}" "${HELM_ARGS[@]}"
    fi

    # Wait for rollout
    print_step "Waiting for deployment to be ready..."
    kubectl rollout status deployment/${RELEASE_NAME}-moatless-vibe -n "${NAMESPACE}" --timeout=300s

    # Show status
    print_step "Deployment complete! Current status:"
    kubectl get pods -n "${NAMESPACE}" -l app.kubernetes.io/name=moatless-vibe
    echo ""
    kubectl get ingress -n "${NAMESPACE}"
fi

echo ""
print_step "Done!"
