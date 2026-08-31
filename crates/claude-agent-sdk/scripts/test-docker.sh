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
#   CLAUDE_CLI_VERSION       - Override the pinned Claude CLI version (default: 2.1.251)
#   SKIP_BUILD=1             - Skip Docker image build step
#   SKIP_COMPILE=1           - Skip Rust cross-compilation step

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$CRATE_DIR/../.." && pwd)"
MODE="${1:-sdk-e2e}"
TEST_FILTER="${2:-}"

# The `sandbox-transport` and `codex-transport` modes run moatless-sandbox agent
# tests inside the real sandbox base image (ships the pinned `claude` CLI, `tmux`
# AND the `codex` CLI). All other modes use the slim Dockerfile.test image.
if [ "$MODE" = "sandbox-transport" ] || [ "$MODE" = "codex-transport" ]; then
    IMAGE_NAME="moatless-sandbox-e2e"
else
    IMAGE_NAME="claude-sdk-test"
fi

# ── Target arch / docker platform ───────────────────────────────────────────
# sandbox-transport runs in the published sandbox base, which is amd64-only, so
# default to amd64 and run under emulation on arm64 hosts (e.g. Apple Silicon).
# Override with SANDBOX_E2E_ARCH=arm64 if you've built a local arm64 base via
# `docker/build-sandbox.sh --local --only base`. Other modes use the multi-arch
# Dockerfile.test base, so match the host arch.
if [ "$MODE" = "sandbox-transport" ] || [ "$MODE" = "codex-transport" ]; then
    DEFAULT_ARCH="amd64"
else
    case "$(uname -m)" in
        x86_64|amd64)  DEFAULT_ARCH="amd64" ;;
        aarch64|arm64) DEFAULT_ARCH="arm64" ;;
        *) DEFAULT_ARCH="amd64" ;;
    esac
fi
ARCH_DIR="${SANDBOX_E2E_ARCH:-$DEFAULT_ARCH}"
case "$ARCH_DIR" in
    amd64) RUST_TARGET="x86_64-unknown-linux-gnu";  DOCKER_PLATFORM="linux/amd64" ;;
    arm64) RUST_TARGET="aarch64-unknown-linux-gnu"; DOCKER_PLATFORM="linux/arm64" ;;
    *) echo "Unsupported SANDBOX_E2E_ARCH: $ARCH_DIR (use amd64 or arm64)"; exit 1 ;;
esac

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

        # RUST_TARGET / ARCH_DIR resolved above (sandbox-transport forces amd64
        # to match the amd64-only sandbox base; other modes match the host).
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

        # Find and copy the both-transports agent test binary (subprocess + tmux)
        AGENT_TRANSPORT_BIN=$(find "$TARGET_DIR/$RUST_TARGET/debug/deps" -name "agent_transport_e2e-*" -type f ! -name "*.d" 2>/dev/null | head -1)
        if [ -n "$AGENT_TRANSPORT_BIN" ]; then
            cp "$AGENT_TRANSPORT_BIN" "$BINARIES_DIR/$ARCH_DIR/agent-transport-e2e"
            echo "  -> agent-transport-e2e"
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
    if [ "$MODE" = "sandbox-transport" ] || [ "$MODE" = "codex-transport" ]; then
        # Run inside the real sandbox base image (pinned claude CLI + tmux).
        # Requires aorwall/moatless-sandbox-base:latest locally — build it with
        # `docker/build-sandbox.sh --local --only base` or override via SANDBOX_BASE.
        docker build \
            --platform "$DOCKER_PLATFORM" \
            -f "$ROOT_DIR/docker/Dockerfile.sandbox-e2e" \
            --build-arg "SANDBOX_BASE=${SANDBOX_BASE:-aorwall/moatless-sandbox-base:latest}" \
            --build-context "binaries=$BINARIES_DIR" \
            -t "$IMAGE_NAME" \
            "$ROOT_DIR"
    else
        docker build \
            --platform "$DOCKER_PLATFORM" \
            -f "$CRATE_DIR/Dockerfile.test" \
            --build-arg "CLAUDE_CLI_VERSION=${CLAUDE_CLI_VERSION:-2.1.251}" \
            --build-context "binaries=$BINARIES_DIR" \
            -t "$IMAGE_NAME" \
            "$ROOT_DIR"
    fi
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
    sandbox-transport)
        # Drives the production spawn_agent path over BOTH transports inside the
        # sandbox base image. Subprocess + tmux tests self-gate on the env vars.
        # --test-threads=1 runs them SERIALLY: the tmux TUI is heavy (especially
        # under amd64 emulation on Apple Silicon) and would otherwise contend
        # with the subprocess test and miss the SessionStart window.
        if [ -n "$TEST_FILTER" ]; then
            TEST_CMD="CLAUDE_SDK_E2E=1 CLAUDE_TMUX_E2E=1 /test-bins/agent-transport-e2e --ignored --nocapture --test-threads=1 $TEST_FILTER"
        else
            TEST_CMD="CLAUDE_SDK_E2E=1 CLAUDE_TMUX_E2E=1 /test-bins/agent-transport-e2e --ignored --nocapture --test-threads=1"
        fi
        ;;
    codex-transport)
        # Drives the production spawn_agent path for Codex (`codex exec --json`
        # subprocess) inside the sandbox base image, which ships the `codex` CLI.
        # Reuses the agent-transport-e2e binary; the codex test self-gates on
        # CODEX_E2E. Serial (--test-threads=1) to keep one live CLI at a time.
        if [ -n "$TEST_FILTER" ]; then
            TEST_CMD="CODEX_E2E=1 /test-bins/agent-transport-e2e --ignored --nocapture --test-threads=1 $TEST_FILTER"
        else
            TEST_CMD="CODEX_E2E=1 /test-bins/agent-transport-e2e --ignored --nocapture --test-threads=1 test_spawn_agent_codex"
        fi
        ;;
    all)
        TEST_CMD="CLAUDE_SDK_E2E=1 /test-bins/sdk-e2e --ignored --nocapture && CLAUDE_SDK_E2E=1 /test-bins/sandbox-e2e --ignored --nocapture"
        ;;
    *)
        echo "Unknown mode: $MODE"
        echo ""
        echo "Usage: $0 {sdk-e2e|smoke|sandbox-e2e|sandbox-transport|codex-transport|all|build} [test_filter]"
        echo ""
        echo "Modes:"
        echo "  sdk-e2e           Run claude-agent-sdk E2E tests (default)"
        echo "  smoke             Run claude-agent-sdk smoke tests (version check + roundtrip + --resume)"
        echo "  sandbox-e2e       Run moatless-sandbox E2E tests"
        echo "  sandbox-transport Run the spawn_agent both-transports test (subprocess + tmux) in the sandbox base image"
        echo "  codex-transport   Run the spawn_agent Codex CLI test (codex exec) in the sandbox base image"
        echo "  all               Run all E2E tests"
        echo "  build             Build Docker image only"
        echo ""
        echo "Examples:"
        echo "  $0 smoke                          # Run smoke tests against the pinned CLI"
        echo "  $0 sdk-e2e test_simple_query      # Run a single test"
        echo "  $0 sandbox-transport              # Both transports in the sandbox image"
        echo "  OPENAI_API_KEY=sk-... $0 codex-transport   # Codex CLI path in the sandbox image"
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
    --platform "$DOCKER_PLATFORM" \
    -e CLAUDE_CODE_OAUTH_TOKEN \
    -e ANTHROPIC_API_KEY \
    -e CLAUDE_SDK_E2E=1 \
    -e CLAUDE_TMUX_E2E \
    -e CODEX_E2E \
    -e OPENAI_API_KEY \
    -e CODEX_AUTH_JSON \
    -e CODEX_TEST_MODEL \
    -e RUST_LOG="${RUST_LOG:-claude_agent_sdk=debug,moatless_sandbox=debug}" \
    "$IMAGE_NAME" \
    "$TEST_CMD"

echo ""
echo "=== Tests complete ==="
