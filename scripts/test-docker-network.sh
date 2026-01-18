#!/bin/bash
#
# Docker Network Architecture Test Script
#
# This script verifies the Docker network changes:
# 1. Sandbox containers attach to moatless network
# 2. Only preview port (3000) is mapped to host
# 3. Backend communicates with sandbox via container name
# 4. Preview URL is direct browser access (http://localhost:{previewHostPort})
#
# Usage: ./scripts/test-docker-network.sh
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test counters
TESTS_PASSED=0
TESTS_FAILED=0

# Configuration
BACKEND_URL="${BACKEND_URL:-http://localhost:8000}"
EXPECTED_NETWORK="${EXPECTED_NETWORK:-docker-compose_moatless}"
TIMEOUT_SECONDS=120

# Cleanup on exit
TASK_ID=""
CONTAINER_NAME=""
cleanup() {
    if [ -n "$TASK_ID" ]; then
        echo -e "\n${YELLOW}Cleaning up...${NC}"
        # Delete task (triggers container cleanup)
        curl -s -X DELETE "$BACKEND_URL/api/v1/tasks/$TASK_ID" > /dev/null 2>&1 || true
        # Force remove container if still exists
        if [ -n "$CONTAINER_NAME" ]; then
            docker rm -f "$CONTAINER_NAME" > /dev/null 2>&1 || true
        fi
        echo -e "${GREEN}Cleanup complete${NC}"
    fi
}
trap cleanup EXIT

# Helper functions
print_header() {
    echo -e "\n${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}\n"
}

print_step() {
    echo -e "${YELLOW}▶ $1${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
}

print_failure() {
    echo -e "${RED}✗ $1${NC}"
    TESTS_FAILED=$((TESTS_FAILED + 1))
}

print_info() {
    echo -e "  ${NC}$1${NC}"
}

wait_with_spinner() {
    local seconds=$1
    local message=$2
    echo -n "  $message "
    for ((i=0; i<seconds; i++)); do
        echo -n "."
        sleep 1
    done
    echo ""
}

# ============================================================================
# PREREQUISITE CHECKS
# ============================================================================

print_header "PREREQUISITE CHECKS"

# Check Docker is running
print_step "Checking Docker daemon..."
if docker info > /dev/null 2>&1; then
    print_success "Docker daemon is running"
else
    print_failure "Docker daemon is not running"
    exit 1
fi

# Check backend is healthy
print_step "Checking backend health..."
HEALTH_RESPONSE=$(curl -s "$BACKEND_URL/health" 2>/dev/null || echo '{"status":"error"}')
HEALTH_STATUS=$(echo "$HEALTH_RESPONSE" | jq -r '.status' 2>/dev/null || echo "error")
if [ "$HEALTH_STATUS" = "ok" ]; then
    print_success "Backend is healthy"
else
    print_failure "Backend is not healthy: $HEALTH_STATUS"
    echo "  Make sure to run: ./docker-compose/dev-docker.sh"
    exit 1
fi

# Check moatless network exists
print_step "Checking Docker network '$EXPECTED_NETWORK'..."
if docker network ls | grep -q "$EXPECTED_NETWORK"; then
    print_success "Network '$EXPECTED_NETWORK' exists"
else
    print_failure "Network '$EXPECTED_NETWORK' not found"
    echo "  Available networks:"
    docker network ls | grep moatless || echo "  No moatless networks found"
    exit 1
fi

# Get template/repository ID
print_step "Getting template repository..."
TEMPLATE_ID=$(curl -s "$BACKEND_URL/api/v1/repositories" | jq -r '.repositories[0].id' 2>/dev/null)
if [ -z "$TEMPLATE_ID" ] || [ "$TEMPLATE_ID" = "null" ]; then
    print_failure "No template repository found"
    exit 1
fi
print_success "Template ID: $TEMPLATE_ID"

# ============================================================================
# TASK CREATION
# ============================================================================

print_header "TASK CREATION"

print_step "Creating new vibe task..."
CREATE_RESPONSE=$(curl -s -X POST "$BACKEND_URL/api/v1/tasks/vibe" \
    -H "Content-Type: application/json" \
    -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Docker network test\",\"start\":false}")

TASK_ID=$(echo "$CREATE_RESPONSE" | jq -r '.taskId' 2>/dev/null)
if [ -z "$TASK_ID" ] || [ "$TASK_ID" = "null" ]; then
    print_failure "Failed to create task"
    echo "  Response: $CREATE_RESPONSE"
    exit 1
fi
print_success "Task created: $TASK_ID"

# Derive container name (first 8 chars of task ID)
CONTAINER_NAME="moatless-moatable-${TASK_ID:0:8}"
print_info "Expected container: $CONTAINER_NAME"

# ============================================================================
# WORKSPACE PROVISIONING
# ============================================================================

print_header "WORKSPACE PROVISIONING"

print_step "Waiting for container to be created..."
ELAPSED=0
CONTAINER_RUNNING=false
while [ $ELAPSED -lt $TIMEOUT_SECONDS ]; do
    if docker ps --format "{{.Names}}" | grep -q "$CONTAINER_NAME"; then
        CONTAINER_STATUS=$(docker inspect "$CONTAINER_NAME" --format '{{.State.Status}}' 2>/dev/null || echo "unknown")
        if [ "$CONTAINER_STATUS" = "running" ]; then
            CONTAINER_RUNNING=true
            break
        fi
    fi
    sleep 2
    ((ELAPSED+=2))
    echo -n "."
done
echo ""

if [ "$CONTAINER_RUNNING" = true ]; then
    print_success "Container is running (took ${ELAPSED}s)"
else
    print_failure "Container did not start within ${TIMEOUT_SECONDS}s"
    echo "  Checking backend logs for errors..."
    docker logs moatless-backend --tail 20 2>&1 | grep -E "(error|Error|$TASK_ID)" | tail -5
    exit 1
fi

# ============================================================================
# NETWORK VERIFICATION
# ============================================================================

print_header "NETWORK VERIFICATION"

# Check container is on correct network
print_step "Verifying container network..."
CONTAINER_NETWORK=$(docker inspect "$CONTAINER_NAME" --format '{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}' 2>/dev/null)
if [ "$CONTAINER_NETWORK" = "$EXPECTED_NETWORK" ]; then
    print_success "Container is on network: $CONTAINER_NETWORK"
else
    print_failure "Container is on wrong network: $CONTAINER_NETWORK (expected: $EXPECTED_NETWORK)"
fi

# Check port mappings
print_step "Verifying port mappings..."
PORT_MAPPINGS=$(docker port "$CONTAINER_NAME" 2>/dev/null)
echo "  Port mappings:"
echo "$PORT_MAPPINGS" | sed 's/^/    /'

# Check that port 3000 is mapped (preview)
if echo "$PORT_MAPPINGS" | grep -q "3000/tcp"; then
    PREVIEW_HOST_PORT=$(docker port "$CONTAINER_NAME" 3000 2>/dev/null | head -1 | cut -d: -f2)
    print_success "Preview port 3000 is mapped to host port $PREVIEW_HOST_PORT"
else
    print_failure "Preview port 3000 is not mapped"
fi

# Check that port 8001 is NOT mapped (sandbox API uses docker network)
if echo "$PORT_MAPPINGS" | grep -q "8001/tcp"; then
    print_failure "Port 8001 should NOT be mapped (backend uses docker network)"
else
    print_success "Port 8001 is not mapped (correct - uses docker network)"
fi

# ============================================================================
# BACKEND-SANDBOX COMMUNICATION
# ============================================================================

print_header "BACKEND-SANDBOX COMMUNICATION"

# Wait for workspace to be ready
print_step "Waiting for workspace to be ready..."
wait_with_spinner 5 "Allowing time for git clone"

# Test file listing (proves backend can communicate with sandbox)
print_step "Testing file operations via backend..."
FILES_RESPONSE=$(curl -s "$BACKEND_URL/api/v1/tasks/$TASK_ID/files?recursive=false")
FILES_COUNT=$(echo "$FILES_RESPONSE" | jq '.files | length' 2>/dev/null || echo "0")
if [ "$FILES_COUNT" -gt 0 ]; then
    print_success "Backend can list files: $FILES_COUNT files found"
else
    print_failure "Backend cannot list files (sandbox communication failed)"
    echo "  Response: $FILES_RESPONSE"
fi

# Test file write
print_step "Testing file write via backend..."
WRITE_RESPONSE=$(curl -s -X PUT "$BACKEND_URL/api/v1/tasks/$TASK_ID/files/docker-network-test.txt" \
    -H "Content-Type: application/json" \
    -d '{"content":"Docker network test - written via backend -> sandbox communication"}')
WRITE_SUCCESS=$(echo "$WRITE_RESPONSE" | jq -r '.success' 2>/dev/null || echo "false")
if [ "$WRITE_SUCCESS" = "true" ]; then
    print_success "File write successful"
else
    print_failure "File write failed"
    echo "  Response: $WRITE_RESPONSE"
fi

# Test file read
print_step "Testing file read via backend..."
READ_RESPONSE=$(curl -s "$BACKEND_URL/api/v1/tasks/$TASK_ID/files/docker-network-test.txt")
READ_CONTENT=$(echo "$READ_RESPONSE" | jq -r '.content' 2>/dev/null || echo "")
if echo "$READ_CONTENT" | grep -q "Docker network test"; then
    print_success "File read successful"
else
    print_failure "File read failed or content mismatch"
    echo "  Response: $READ_RESPONSE"
fi

# ============================================================================
# PREVIEW SERVER VERIFICATION
# ============================================================================

print_header "PREVIEW SERVER VERIFICATION"

# Start preview server if not running
print_step "Starting preview server..."
curl -s -X POST "$BACKEND_URL/api/v1/tasks/$TASK_ID/server/start" > /dev/null 2>&1

# Wait for preview server to start
print_step "Waiting for preview server to start..."
PREVIEW_WAIT=0
PREVIEW_MAX_WAIT=60
while [ $PREVIEW_WAIT -lt $PREVIEW_MAX_WAIT ]; do
    SERVER_STATUS=$(curl -s "$BACKEND_URL/api/v1/tasks/$TASK_ID/server/status")
    PREVIEW_STATUS=$(echo "$SERVER_STATUS" | jq -r '.status' 2>/dev/null || echo "unknown")
    if [ "$PREVIEW_STATUS" = "started" ]; then
        break
    fi
    sleep 2
    ((PREVIEW_WAIT+=2))
    echo -n "."
done
echo ""

# Check preview server status
print_step "Checking preview server status..."
SERVER_STATUS=$(curl -s "$BACKEND_URL/api/v1/tasks/$TASK_ID/server/status")
PREVIEW_STATUS=$(echo "$SERVER_STATUS" | jq -r '.status' 2>/dev/null || echo "unknown")
PREVIEW_URL=$(echo "$SERVER_STATUS" | jq -r '.url' 2>/dev/null || echo "")

print_info "Status: $PREVIEW_STATUS"
print_info "URL: $PREVIEW_URL"

# Verify preview URL format
print_step "Verifying preview URL format..."
if echo "$PREVIEW_URL" | grep -qE "^http://localhost:[0-9]+$"; then
    print_success "Preview URL is direct browser access: $PREVIEW_URL"
else
    print_failure "Preview URL format incorrect: $PREVIEW_URL"
    echo "  Expected: http://localhost:{port}"
fi

# Verify preview URL uses correct port
if [ -n "$PREVIEW_HOST_PORT" ]; then
    if echo "$PREVIEW_URL" | grep -q ":$PREVIEW_HOST_PORT"; then
        print_success "Preview URL uses correct host port: $PREVIEW_HOST_PORT"
    else
        print_failure "Preview URL port mismatch"
        echo "  Expected port: $PREVIEW_HOST_PORT"
        echo "  URL: $PREVIEW_URL"
    fi
fi

# Test preview server accessibility
print_step "Testing preview server accessibility..."
if [ -n "$PREVIEW_HOST_PORT" ]; then
    PREVIEW_HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:$PREVIEW_HOST_PORT/" --connect-timeout 5 2>/dev/null || echo "000")
    if [ "$PREVIEW_HTTP_CODE" -ge 200 ] && [ "$PREVIEW_HTTP_CODE" -lt 500 ]; then
        print_success "Preview server is accessible (HTTP $PREVIEW_HTTP_CODE)"
    else
        print_failure "Preview server not accessible (HTTP $PREVIEW_HTTP_CODE)"
    fi
else
    print_failure "Cannot test preview accessibility - no host port found"
fi

# ============================================================================
# INTERNAL COMMUNICATION TEST
# ============================================================================

print_header "INTERNAL COMMUNICATION TEST"

# Test that backend can reach sandbox via container name
# We verify this by checking that file operations work (which requires backend->sandbox communication)
print_step "Verifying internal communication (backend -> sandbox via container name)..."

# The fact that file operations worked earlier proves backend can communicate with sandbox
# Let's do one more explicit test by reading package.json
INTERNAL_TEST=$(curl -s "$BACKEND_URL/api/v1/tasks/$TASK_ID/files/package.json")
INTERNAL_CONTENT=$(echo "$INTERNAL_TEST" | jq -r '.content' 2>/dev/null || echo "")
if echo "$INTERNAL_CONTENT" | grep -q "name"; then
    print_success "Backend communicates with sandbox via container name (verified via file read)"
    print_info "Container URL: http://$CONTAINER_NAME:8001"
else
    print_failure "Backend cannot communicate with sandbox"
    echo "  Response: $INTERNAL_TEST"
fi

# Additional verification: check that sandbox health endpoint works through backend
print_step "Testing sandbox health via backend proxy..."
# The workspace-status endpoint checks sandbox health internally
WORKSPACE_STATUS=$(curl -s "$BACKEND_URL/api/v1/tasks/$TASK_ID/workspace-status")
CAPABILITIES=$(echo "$WORKSPACE_STATUS" | jq '.capabilities' 2>/dev/null)
FILE_OPS=$(echo "$CAPABILITIES" | jq -r '.fileOperations' 2>/dev/null || echo "false")
if [ "$FILE_OPS" = "true" ]; then
    print_success "Sandbox is healthy and accessible from backend"
else
    print_failure "Sandbox health check failed"
    echo "  Capabilities: $CAPABILITIES"
fi

# ============================================================================
# SUMMARY
# ============================================================================

print_header "TEST SUMMARY"

TOTAL_TESTS=$((TESTS_PASSED + TESTS_FAILED))
echo -e "  ${GREEN}Passed: $TESTS_PASSED${NC}"
echo -e "  ${RED}Failed: $TESTS_FAILED${NC}"
echo -e "  Total:  $TOTAL_TESTS"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║                    ALL TESTS PASSED!                          ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo "Docker Network Architecture verified:"
    echo "  - Sandbox containers attach to $EXPECTED_NETWORK"
    echo "  - Only preview port (3000) is mapped to host"
    echo "  - Backend communicates with sandbox via container name"
    echo "  - Preview URL is direct browser access"
    exit 0
else
    echo -e "${RED}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║                    SOME TESTS FAILED                          ║${NC}"
    echo -e "${RED}╚═══════════════════════════════════════════════════════════════╝${NC}"
    exit 1
fi
