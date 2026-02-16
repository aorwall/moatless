# Moatless Vibe - Deployment Tasks
# Usage: make <target>

# Configuration
NAMESPACE ?= vibe
RELEASE_NAME ?= vibe
TAG ?= latest
PLATFORM ?= linux/amd64
HELM_CHART := k8s/helm/moatless-vibe

# Colors
GREEN := \033[0;32m
YELLOW := \033[1;33m
NC := \033[0m

.PHONY: help build-app build-worker build deploy-app deploy-worker deploy logs status restart test-integration

## Help
help: ## Show this help
	@echo "Usage: make [target]"
	@echo ""
	@echo "Deployment targets:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(GREEN)%-20s$(NC) %s\n", $$1, $$2}'
	@echo ""
	@echo "Variables:"
	@echo "  TAG=v1.0.0      Docker image tag (default: latest)"
	@echo "  NAMESPACE=vibe  Kubernetes namespace (default: vibe)"
	@echo ""
	@echo "Examples:"
	@echo "  make deploy              # Build and deploy everything"
	@echo "  make deploy-app          # Build and deploy app only"
	@echo "  make deploy TAG=v2.0.0   # Deploy with specific tag"

## Build targets
build-app: ## Build app Docker image
	@echo "$(GREEN)==> Building app image...$(NC)"
	./docker/build-app.sh --tag $(TAG) --platform $(PLATFORM)

build-worker: ## Build worker Docker image
	@echo "$(GREEN)==> Building worker image...$(NC)"
	./docker/build-worker.sh --tag $(TAG) --platform $(PLATFORM)

build: build-app build-worker ## Build both Docker images

## Push targets
push-app: ## Build and push app image
	@echo "$(GREEN)==> Building and pushing app image...$(NC)"
	./docker/build-app.sh --tag $(TAG) --platform $(PLATFORM) --push

push-worker: ## Build and push worker image
	@echo "$(GREEN)==> Building and pushing worker image...$(NC)"
	./docker/build-worker.sh --tag $(TAG) --platform $(PLATFORM) --push

push: push-app push-worker ## Build and push both images

## Deploy targets
deploy-app: push-app ## Build, push, and deploy app only
	@echo "$(GREEN)==> Restarting app deployment...$(NC)"
	kubectl rollout restart deployment/$(RELEASE_NAME)-moatless-vibe -n $(NAMESPACE)
	kubectl rollout status deployment/$(RELEASE_NAME)-moatless-vibe -n $(NAMESPACE) --timeout=300s
	@echo "$(GREEN)==> App deployed successfully!$(NC)"

deploy-worker: push-worker ## Build, push worker (new pods use new image)
	@echo "$(GREEN)==> Worker image pushed. New worker pods will use the updated image.$(NC)"
	@echo "$(YELLOW)Note: Existing worker pods will continue until task completion.$(NC)"

deploy: push ## Build, push both, and restart app
	@echo "$(GREEN)==> Upgrading Helm release...$(NC)"
	helm upgrade $(RELEASE_NAME) $(HELM_CHART) \
		--namespace $(NAMESPACE) \
		--set image.tag=$(TAG) \
		--set workerImage.tag=$(TAG) \
		--set externalSecrets.enabled=true
	kubectl rollout status deployment/$(RELEASE_NAME)-moatless-vibe -n $(NAMESPACE) --timeout=300s
	@echo "$(GREEN)==> Deployment complete!$(NC)"

## Kubernetes operations
restart: ## Restart the app deployment (pull latest image)
	@echo "$(GREEN)==> Restarting deployment...$(NC)"
	kubectl rollout restart deployment/$(RELEASE_NAME)-moatless-vibe -n $(NAMESPACE)
	kubectl rollout status deployment/$(RELEASE_NAME)-moatless-vibe -n $(NAMESPACE) --timeout=300s

status: ## Show deployment status
	@echo "$(GREEN)==> Pods:$(NC)"
	kubectl get pods -n $(NAMESPACE) -l app.kubernetes.io/name=moatless-vibe
	@echo ""
	@echo "$(GREEN)==> Deployment:$(NC)"
	kubectl get deployment -n $(NAMESPACE) -l app.kubernetes.io/name=moatless-vibe
	@echo ""
	@echo "$(GREEN)==> Worker pods:$(NC)"
	kubectl get pods -n $(NAMESPACE) -l app.kubernetes.io/component=worker 2>/dev/null || echo "No worker pods running"

logs: ## Show app logs
	kubectl logs -n $(NAMESPACE) deployment/$(RELEASE_NAME)-moatless-vibe --tail=100 -f

logs-worker: ## Show logs for all worker pods
	@echo "$(GREEN)==> Worker pods:$(NC)"
	@kubectl get pods -n $(NAMESPACE) -l app.kubernetes.io/component=worker --no-headers | while read pod rest; do \
		echo "$(YELLOW)--- $$pod ---$(NC)"; \
		kubectl logs -n $(NAMESPACE) $$pod --tail=20 2>/dev/null || echo "No logs available"; \
	done

## Database
db-migrate: ## Run database migrations (via app restart)
	@echo "$(GREEN)==> Migrations run automatically on app startup$(NC)"
	@echo "Restarting app to trigger migrations..."
	$(MAKE) restart

## Testing
test-integration: ## Run backend integration tests (requires DATABASE_URL)
	cargo test -p moatless-vibe-backend --features db-tests

## Cleanup
clean-workers: ## Delete all worker pods
	@echo "$(YELLOW)==> Deleting all worker pods...$(NC)"
	kubectl delete pods -n $(NAMESPACE) -l app.kubernetes.io/component=worker --force --grace-period=0 2>/dev/null || echo "No worker pods to delete"
