#!/bin/bash
# Run Rust SDK E2E tests in a Docker container.
# Cross-compiles test binaries on host, then builds a slim Docker image.
# Verifies that CLAUDE_CODE_OAUTH_TOKEN works inside Docker.
#
# Usage:
#   ./crates/claude-agent-sdk/scripts/test-docker.sh                        # Run SDK E2E tests (default)
#   ./crates/claude-agent-sdk/scripts/test-docker.sh sdk-e2e                # Run claude-agent-sdk E2E tests
#   ./crates/claude-agent-sdk/scripts/test-docker.sh sdk-e2e test_simple_query  # Run a single SDK test
#   ./crates/claude-agent-sdk/scripts/test-docker.sh smoke                  # Run claude-agent-sdk smoke tests
#   ./crates/claude-agent-sdk/scripts/test-docker.sh sandbox-e2e            # Run moatless-sandbox E2E tests
#   ./crates/claude-agent-sdk/scripts/test-docker.sh all                    # Run all E2E tests
#   ./crates/claude-agent-sdk/scripts/test-docker.sh build                  # Build image only
#
# Environment:
#   CLAUDE_CODE_OAUTH_TOKEN  - OAuth token for Claude API (preferred)
#   ANTHROPIC_API_KEY        - API key for Claude API (alternative)
#   CLAUDE_CLI_VERSION       - Override the pinned Claude CLI version (default: 2.1.145)
#   SKIP_BUILD=1             - Skip Docker image build step
#   SKIP_COMPILE=1           - Skip Rust cross-compilation step

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$CRATE_DIR/../.." && pwd)"
MODE="${1:-sdk-e2e}"
TEST_FILTER="${2:-}"
IMAGE_NAME="claude-sdk-test"

cd "$ROOT_DIR"

# ── Auth tokens ───────────────────────────────────────────────────────────
# Tokens are passed through from the host environment via `docker run -e`.
# The test code reads them from its environment and forwards via ClaudeOptions.env.
# No .env loading or validation needed here — skip_unless_e2e!() gates test execution.

# ── Cross-compile test binaries for Linux ───────────────────────────────────

BINARIES_DIR=""
cleanup() {
    if [ -n "$BINARIES_DIR" ] && [ -d "$BINARIES_DIR" ]; then
        rm -rf "$BINARIES_DIR"
    fi
}
trap cleanup EXIT

if [ "${SKIP_BUILD:-0}" != "1" ]; then
    if [ "${SKIP_COMPILE:-0}" != "1" ]; then
        if ! command -v cargo-zigbuild &> /dev/null; then
            echo "Error: cargo-zigbuild required for cross-compilation."
            echo "Install: cargo install cargo-zigbuild && pip3 install ziglang"
            exit 1
        fi

        # Zig linker needs more file descriptors
        ulimit -n 10240 2>/dev/null || true

        TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
            | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"

        # Determine target architecture (Docker runs Linux)
        case "$(uname -m)" in
            x86_64|amd64)  RUST_TARGET="x86_64-unknown-linux-gnu"; ARCH_DIR="amd64" ;;
            aarch64|arm64) RUST_TARGET="aarch64-unknown-linux-gnu"; ARCH_DIR="arm64" ;;
            *) echo "Unsupported host architecture: $(uname -m)"; exit 1 ;;
        esac

        BINARIES_DIR="$(mktemp -d)"
        mkdir -p "$BINARIES_DIR/$ARCH_DIR"

        echo ""
        echo "=== Cross-compiling test binaries for $RUST_TARGET ==="

        # Build SDK E2E test binary
        echo "Compiling claude-agent-sdk tests..."
        cargo zigbuild --tests -p claude-agent-sdk --target "$RUST_TARGET" 2>&1

        # Find and copy the SDK e2e test binary (exclude .d dep-info files)
        SDK_E2E_BIN=$(find "$TARGET_DIR/$RUST_TARGET/debug/deps" -name "e2e-*" -type f ! -name "*.d" 2>/dev/null | head -1)
        if [ -n "$SDK_E2E_BIN" ]; then
            cp "$SDK_E2E_BIN" "$BINARIES_DIR/$ARCH_DIR/sdk-e2e"
            echo "  -> sdk-e2e"
        else
            echo "Warning: SDK E2E test binary not found"
        fi

        # Find and copy the SDK smoke test binary (exclude .d dep-info files)
        SDK_SMOKE_BIN=$(find "$TARGET_DIR/$RUST_TARGET/debug/deps" -name "smoke-*" -type f ! -name "*.d" 2>/dev/null | head -1)
        if [ -n "$SDK_SMOKE_BIN" ]; then
            cp "$SDK_SMOKE_BIN" "$BINARIES_DIR/$ARCH_DIR/smoke"
            echo "  -> smoke"
        else
            echo "Warning: SDK smoke test binary not found"
        fi

        # Build sandbox E2E test binary
        echo "Compiling moatless-sandbox tests..."
        cargo zigbuild --tests -p moatless-sandbox --target "$RUST_TARGET" 2>&1

        # Find and copy the sandbox e2e test binary (exclude .d dep-info files)
        SANDBOX_E2E_BIN=$(find "$TARGET_DIR/$RUST_TARGET/debug/deps" -name "agent_e2e-*" -type f ! -name "*.d" 2>/dev/null | head -1)
        if [ -n "$SANDBOX_E2E_BIN" ]; then
            cp "$SANDBOX_E2E_BIN" "$BINARIES_DIR/$ARCH_DIR/sandbox-e2e"
            echo "  -> sandbox-e2e"
        fi

        echo ""
        echo "Binaries staged in $BINARIES_DIR/$ARCH_DIR/:"
        ls -lh "$BINARIES_DIR/$ARCH_DIR/"
        echo "=== Compilation complete ==="
    fi

    # ── Build Docker image ──────────────────────────────────────────────────

    # If SKIP_COMPILE was set, look for previously staged binaries
    if [ -z "$BINARIES_DIR" ]; then
        echo "Error: No binaries directory. Run without SKIP_COMPILE first."
        exit 1
    fi

    echo ""
    echo "=== Building Docker test image: $IMAGE_NAME ==="
    docker build \
        -f "$CRATE_DIR/Dockerfile.test" \
        --build-arg "CLAUDE_CLI_VERSION=${CLAUDE_CLI_VERSION:-2.1.145}" \
        --build-context "binaries=$BINARIES_DIR" \
        -t "$IMAGE_NAME" \
        "$ROOT_DIR"
    echo "=== Build complete ==="
fi

if [ "$MODE" = "build" ]; then
    echo "Build-only mode, exiting."
    exit 0
fi

# ── Compose test command ───────────────────────────────────────────────────

case "$MODE" in
    sdk-e2e)
        if [ -n "$TEST_FILTER" ]; then
            TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/sdk-e2e --ignored --nocapture $TEST_FILTER"
        else
            TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/sdk-e2e --ignored --nocapture"
        fi
        ;;
    smoke)
        if [ -n "$TEST_FILTER" ]; then
            TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/smoke --ignored --nocapture $TEST_FILTER"
        else
            TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/smoke --ignored --nocapture"
        fi
        ;;
    sandbox-e2e)
        if [ -n "$TEST_FILTER" ]; then
            TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/sandbox-e2e --ignored --nocapture $TEST_FILTER"
        else
            TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/sandbox-e2e --ignored --nocapture"
        fi
        ;;
    all)
        TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/sdk-e2e --ignored --nocapture && CLAUDE_SDK_E2E=1 /test-bins/sandbox-e2e --ignored --nocapture"
        ;;
    *)
        echo "Unknown mode: $MODE"
        echo ""
        echo "Usage: $0 {sdk-e2e|smoke|sandbox-e2e|all|build} [test_filter]"
        echo ""
        echo "Modes:"
        echo "  sdk-e2e      Run claude-agent-sdk E2E tests (default)"
        echo "  smoke        Run claude-agent-sdk smoke tests (version check + roundtrip + --resume)"
        echo "  sandbox-e2e  Run moatless-sandbox E2E tests"
        echo "  all          Run all E2E tests"
        echo "  build        Build Docker image only"
        echo ""
        echo "Examples:"
        echo "  $0 smoke                          # Run smoke tests against the pinned CLI"
        echo "  $0 sdk-e2e test_simple_query      # Run a single test"
        echo "  SKIP_BUILD=1 $0 sdk-e2e           # Skip image rebuild"
        exit 1
        ;;
esac

# ── Run tests ──────────────────────────────────────────────────────────────

echo ""
echo "=== Running tests: $MODE ==="
echo "Command: $TEST_CMD"
echo ""

docker run --rm \
    -e CLAUDE_CODE_OAUTH_TOKEN \
    -e ANTHROPIC_API_KEY \
    -e CLAUDE_SDK_E2E=1 \
    -e RUST_LOG="${RUST_LOG:-claude_agent_sdk=debug,moatless_sandbox=debug}" \
    "$IMAGE_NAME" \
    "$TEST_CMD"

echo ""
echo "=== Tests complete ==="
