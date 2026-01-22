#!/bin/bash
set -e

# Default configuration
LOG_LEVEL="${LOG_LEVEL:-info}"
FOLLOW_LOGS="${FOLLOW_LOGS:-true}"
PRETTY="${PRETTY:-true}"

# Parse command-line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --log-level)
      LOG_LEVEL="$2"
      shift 2
      ;;
    --no-logs)
      FOLLOW_LOGS="false"
      shift
      ;;
    --raw|-r)
      PRETTY="false"
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --log-level LEVEL   Set log level filter (debug, info, warn, error). Default: info"
      echo "  --no-logs           Don't stream logs after starting containers"
      echo "  -r, --raw           Show raw JSON logs without formatting (default: pretty)"
      echo "  -h, --help          Show this help message"
      echo ""
      echo "Environment variables:"
      echo "  LOG_LEVEL           Same as --log-level"
      echo "  FOLLOW_LOGS         Set to 'false' to disable log streaming"
      echo "  PRETTY              Set to 'false' for raw JSON output"
      exit 0
      ;;
    *)
      echo "Unknown option: $1"
      echo "Use --help for usage information"
      exit 1
      ;;
  esac
done

# Change to the project root directory
cd "$(dirname "$0")/.."

echo "=========================================="
echo "Stopping existing moatless containers..."
echo "=========================================="

# Stop docker-compose services first
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  down --remove-orphans 2>/dev/null || true

# Stop all sandbox containers (labeled with created-by=moatless)
SANDBOX_CONTAINERS=$(docker ps -q --filter "label=created-by=moatless" 2>/dev/null || true)
if [ -n "$SANDBOX_CONTAINERS" ]; then
  echo "Stopping sandbox containers..."
  echo "$SANDBOX_CONTAINERS" | xargs docker stop 2>/dev/null || true
fi

SANDBOX_CONTAINERS_STOPPED=$(docker ps -aq --filter "label=created-by=moatless" 2>/dev/null || true)
if [ -n "$SANDBOX_CONTAINERS_STOPPED" ]; then
  echo "Removing stopped sandbox containers..."
  echo "$SANDBOX_CONTAINERS_STOPPED" | xargs docker rm 2>/dev/null || true
fi

# Also stop any containers with moatless- prefix (catch-all for unlabeled containers)
MOATLESS_CONTAINERS=$(docker ps -q --filter "name=moatless-" 2>/dev/null || true)
if [ -n "$MOATLESS_CONTAINERS" ]; then
  echo "Stopping remaining moatless-* containers..."
  echo "$MOATLESS_CONTAINERS" | xargs docker stop 2>/dev/null || true
fi

echo ""
echo "=========================================="
echo "Building application..."
echo "=========================================="
bun run build:package

echo ""
echo "=========================================="
echo "Building local sandbox base image..."
echo "=========================================="
docker build -t moatless-vibe-sandbox:local -f docker/Dockerfile.sandbox .

echo ""
echo "=========================================="
echo "Building local sandbox-js image..."
echo "=========================================="
docker build -t moatless-vibe-sandbox-js:local \
  --build-arg BASE_IMAGE=moatless-vibe-sandbox:local \
  -f docker/Dockerfile.sandbox-js .

# Export sandbox image for docker-compose to use
export SANDBOX_IMAGE=moatless-vibe-sandbox-js:local

echo ""
echo "=========================================="
echo "Starting docker compose..."
echo "=========================================="
# Use desktop-linux builder which can access local images
docker buildx use desktop-linux 2>/dev/null || true
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  --env-file docker-compose/.env.docker-compose \
  up -d --build

echo ""
echo "=========================================="
echo "Done! Backend running at http://localhost:8000"
echo "=========================================="

# Filter function for log levels
filter_logs() {
  while IFS= read -r line; do
    # Check if this is a JSON log line (contains "level")
    if echo "$line" | grep -q '"level"'; then
      case "$LOG_LEVEL" in
        debug)
          # Show all levels
          echo "$line"
          ;;
        info)
          # Show info, warn, error (exclude debug)
          if ! echo "$line" | grep -q '"level":"debug"'; then
            echo "$line"
          fi
          ;;
        warn)
          # Show warn and error only
          if echo "$line" | grep -qE '"level":"(warn|error)"'; then
            echo "$line"
          fi
          ;;
        error)
          # Show error only
          if echo "$line" | grep -q '"level":"error"'; then
            echo "$line"
          fi
          ;;
        *)
          # Unknown level, show all
          echo "$line"
          ;;
      esac
    else
      # Non-JSON logs (postgres, redis startup messages) - always show
      echo "$line"
    fi
  done
}

# Pretty print function for JSON logs
pretty_print() {
  while IFS= read -r line; do
    # Extract container name prefix (e.g., "moatless-backend   | ")
    if [[ "$line" =~ ^([a-zA-Z0-9_-]+)[[:space:]]*\|[[:space:]]*(.*) ]]; then
      container="${BASH_REMATCH[1]}"
      json="${BASH_REMATCH[2]}"

      # Shorten container name (remove moatless- prefix)
      short_name="${container#moatless-}"

      # Try to parse as JSON with jq
      if parsed=$(echo "$json" | jq -r '
        # Format timestamp (ms to HH:MM:SS)
        ((.time // 0) / 1000 | strflocaltime("%H:%M:%S")) as $ts |
        # Format level uppercase with padding
        ((.level // "unknown") | ascii_upcase | if length < 5 then . + " " else . end) as $lvl |
        # Get message
        (.msg // "") as $msg |
        # Get extra fields (exclude standard pino fields)
        (to_entries | map(select(.key | IN("level","time","pid","hostname","msg") | not))) as $extras |
        # Build output
        { ts: $ts, lvl: $lvl, msg: $msg, extras: $extras, level: .level }
        | @json
      ' 2>/dev/null); then
        # Extract parsed values
        ts=$(echo "$parsed" | jq -r '.ts')
        lvl=$(echo "$parsed" | jq -r '.lvl')
        msg=$(echo "$parsed" | jq -r '.msg')
        level=$(echo "$parsed" | jq -r '.level')
        extras=$(echo "$parsed" | jq -r '.extras | map("\(.key)=\(.value)") | join(" ")')

        # Color based on level
        case "$level" in
          error) color="\033[31m" ;;  # red
          warn)  color="\033[33m" ;;  # yellow
          info)  color="\033[32m" ;;  # green
          debug) color="\033[36m" ;;  # cyan
          *)     color="" ;;
        esac
        reset="\033[0m"
        dim="\033[2m"

        # Print formatted output
        printf "%-8s | ${color}%s %s${reset} %s\n" "$short_name" "$ts" "$lvl" "$msg"
        if [ -n "$extras" ]; then
          printf "%-8s   ${dim}%s${reset}\n" "" "$extras"
        fi
      else
        # Not valid JSON, pass through as-is
        echo "$line"
      fi
    else
      # No container prefix, pass through
      echo "$line"
    fi
  done
}

# Stream logs if enabled
if [ "$FOLLOW_LOGS" = "true" ]; then
  echo ""
  if [ "$PRETTY" = "true" ]; then
    echo "Streaming logs (level: $LOG_LEVEL, pretty). Press Ctrl+C to stop."
  else
    echo "Streaming logs (level: $LOG_LEVEL, raw). Press Ctrl+C to stop."
  fi
  echo "Use './docker-compose/logs.sh --help' for more log viewing options."
  echo "=========================================="
  echo ""

  # Stream logs with container name prefix, level filtering, and pretty printing
  if [ "$PRETTY" = "true" ]; then
    docker compose -f docker-compose/docker-compose.yml \
      -f docker-compose/docker-compose.dev.yml \
      logs -f 2>&1 | filter_logs | pretty_print
  else
    docker compose -f docker-compose/docker-compose.yml \
      -f docker-compose/docker-compose.dev.yml \
      logs -f 2>&1 | filter_logs
  fi
fi
