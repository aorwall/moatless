#!/bin/bash
# Detect frozen Claude Code processes
# Signs of frozen: 0% CPU, foreground (S+), no network connections, running > 5 min

THRESHOLD_MINUTES=5

echo "Checking for potentially frozen Claude processes..."

for pid in $(pgrep -x claude); do
    # Get process info
    stat=$(ps -p "$pid" -o stat= 2>/dev/null)
    cpu=$(ps -p "$pid" -o %cpu= 2>/dev/null | tr -d ' ')
    etime=$(ps -p "$pid" -o etime= 2>/dev/null | tr -d ' ')
    tty=$(ps -p "$pid" -o tty= 2>/dev/null | tr -d ' ')
    args=$(ps -p "$pid" -o args= 2>/dev/null)

    # Skip if not in foreground (S+)
    [[ "$stat" != *"+"* ]] && continue

    # Check if CPU is essentially 0
    cpu_int=${cpu%.*}
    [[ -z "$cpu_int" ]] && cpu_int=0
    [[ "$cpu_int" -gt 1 ]] && continue

    # Check for network connections
    net_conns=$(lsof -p "$pid" -i 2>/dev/null | grep -c TCP)

    # Count config file handles (leak indicator)
    config_handles=$(lsof -p "$pid" 2>/dev/null | grep -c "\.config\.json")

    # Parse elapsed time (formats: MM:SS, HH:MM:SS, D-HH:MM:SS)
    # Use 10# prefix to force base-10 interpretation (avoid octal issues with 08, 09)
    if [[ "$etime" =~ ^([0-9]+)-.*$ ]]; then
        days=$((10#${BASH_REMATCH[1]}))
        minutes=$((days * 24 * 60))
    elif [[ "$etime" =~ ^([0-9]+):([0-9]+):([0-9]+)$ ]]; then
        minutes=$((10#${BASH_REMATCH[1]} * 60 + 10#${BASH_REMATCH[2]}))
    elif [[ "$etime" =~ ^([0-9]+):([0-9]+)$ ]]; then
        minutes=$((10#${BASH_REMATCH[1]}))
    else
        minutes=0
    fi

    # Flag as potentially frozen
    if [[ "$minutes" -ge "$THRESHOLD_MINUTES" && "$net_conns" -eq 0 ]]; then
        echo ""
        echo "⚠️  POTENTIALLY FROZEN: PID $pid"
        echo "   TTY: $tty | CPU: $cpu% | Running: $etime"
        echo "   Args: $args"
        echo "   Network connections: $net_conns"
        echo "   Config file handles: $config_handles (>20 = likely leak)"
        echo "   Kill with: kill -9 $pid"
    fi
done

echo ""
echo "Done."
