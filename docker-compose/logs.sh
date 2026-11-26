#!/bin/bash
# Moatless log viewer with level filtering and pretty printing
# Usage: ./logs.sh [OPTIONS]

set -e

# Default configuration
LOG_LEVEL="${LOG_LEVEL:-info}"
SERVICE=""
TAIL_LINES=""
PRETTY="${PRETTY:-true}"

# Parse command-line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --level|-l)
      LOG_LEVEL="$2"
      shift 2
      ;;
    --service|-s)
      SERVICE="$2"
      shift 2
      ;;
    --tail|-n)
      TAIL_LINES="$2"
      shift 2
      ;;
    --raw|-r)
      PRETTY="false"
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "View logs from moatless docker-compose services with level filtering."
      echo ""
      echo "Options:"
      echo "  -l, --level LEVEL    Filter by log level (debug, info, warn, error). Default: info"
      echo "  -s, --service NAME   Show logs for specific service (backend, postgres)"
      echo "  -n, --tail LINES     Number of lines to show from the end (default: all)"
      echo "  -r, --raw            Show raw JSON logs without formatting (default: pretty)"
      echo "  -h, --help           Show this help message"
      echo ""
      echo "Environment variables:"
      echo "  LOG_LEVEL            Same as --level"
      echo "  PRETTY               Set to 'false' for raw JSON output"
      echo ""
      echo "Examples:"
      echo "  ./logs.sh                          # Stream all logs at info level (pretty)"
      echo "  ./logs.sh --raw                    # Stream raw JSON logs"
      echo "  ./logs.sh --level debug            # Include debug logs"
      echo "  ./logs.sh --level warn             # Only warn and error"
      echo "  ./logs.sh --service backend        # Only backend logs"
      echo "  ./logs.sh --tail 100               # Show last 100 lines then follow"
      echo "  ./logs.sh -l error -s backend      # Only backend errors"
      echo ""
      echo "Finding moatless containers:"
      echo "  docker ps --filter 'label=created-by=moatless'"
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

# Build docker-compose command
COMPOSE_CMD="docker compose -f docker-compose/docker-compose.yml logs -f"

if [ -n "$TAIL_LINES" ]; then
  COMPOSE_CMD="$COMPOSE_CMD --tail $TAIL_LINES"
fi

if [ -n "$SERVICE" ]; then
  COMPOSE_CMD="$COMPOSE_CMD $SERVICE"
else
  # Default: skip dozzle, whose own logs are noise
  COMPOSE_CMD="$COMPOSE_CMD backend postgres"
fi

if [ "$PRETTY" = "true" ]; then
  echo "Streaming logs (level: $LOG_LEVEL, pretty). Press Ctrl+C to stop."
else
  echo "Streaming logs (level: $LOG_LEVEL, raw). Press Ctrl+C to stop."
fi
echo "=========================================="
echo ""

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
      # Non-JSON logs (postgres startup messages) - always show
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

# Execute and filter
if [ "$PRETTY" = "true" ]; then
  $COMPOSE_CMD 2>&1 | filter_logs | pretty_print
else
  $COMPOSE_CMD 2>&1 | filter_logs
fi
