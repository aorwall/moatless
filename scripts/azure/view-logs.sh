#!/bin/bash

# Log Viewer Script for Moatless Vibe
# This script provides various options for viewing application logs

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Functions
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_option() {
    echo -e "${CYAN}$1${NC}"
}

print_header() {
    echo -e "${BLUE}=========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}=========================================${NC}"
}

# Show menu
show_menu() {
    print_header "Moatless Vibe Log Viewer"
    echo
    print_option "1) View live logs (follow mode)"
    print_option "2) View last 50 lines"
    print_option "3) View last 100 lines"
    print_option "4) View last 500 lines"
    print_option "5) View logs from last hour"
    print_option "6) View logs from today"
    print_option "7) View error logs only"
    print_option "8) View warning and error logs"
    print_option "9) Search logs for specific text"
    print_option "10) View service status"
    print_option "11) View Docker container logs"
    print_option "12) Export logs to file"
    print_option "0) Exit"
    echo
}

# View live logs
view_live_logs() {
    print_header "Live Logs (Press Ctrl+C to stop)"
    sudo journalctl -u moatless-vibe -f
}

# View last N lines
view_last_lines() {
    local lines=$1
    print_header "Last $lines lines of logs"
    sudo journalctl -u moatless-vibe -n "$lines" --no-pager
}

# View logs from time period
view_time_period() {
    local since=$1
    print_header "Logs from $since"
    sudo journalctl -u moatless-vibe --since "$since" --no-pager
}

# View error logs
view_error_logs() {
    print_header "Error Logs"
    sudo journalctl -u moatless-vibe -p err --no-pager
}

# View warning and error logs
view_warning_error_logs() {
    print_header "Warning and Error Logs"
    sudo journalctl -u moatless-vibe -p warning --no-pager
}

# Search logs
search_logs() {
    echo -n "Enter search text: "
    read -r search_text
    print_header "Searching for: $search_text"
    sudo journalctl -u moatless-vibe --no-pager | grep -i "$search_text"
}

# View service status
view_service_status() {
    print_header "Service Status"
    sudo systemctl status moatless-vibe --no-pager
    echo
    print_info "Service Information:"
    echo "  Active: $(systemctl is-active moatless-vibe)"
    echo "  Enabled: $(systemctl is-enabled moatless-vibe)"
    echo "  Running since: $(systemctl show moatless-vibe -p ActiveEnterTimestamp --value)"
    echo "  Main PID: $(systemctl show moatless-vibe -p MainPID --value)"
    echo "  Memory: $(systemctl show moatless-vibe -p MemoryCurrent --value | numfmt --to=iec-i --suffix=B)"
}

# View Docker container logs
view_docker_logs() {
    print_header "Docker Container Logs"
    
    # Check if Docker is running
    if ! systemctl is-active --quiet docker; then
        print_info "Docker is not running"
        return
    fi
    
    # List running containers
    CONTAINERS=$(sudo docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Image}}" | tail -n +2)
    
    if [ -z "$CONTAINERS" ]; then
        print_info "No running Docker containers"
        return
    fi
    
    echo "Running containers:"
    echo "$CONTAINERS"
    echo
    echo -n "Enter container name to view logs (or press Enter to skip): "
    read -r container_name
    
    if [ -n "$container_name" ]; then
        print_header "Logs for container: $container_name"
        sudo docker logs --tail 50 "$container_name"
    fi
}

# Export logs to file
export_logs() {
    local timestamp=$(date +%Y%m%d_%H%M%S)
    local output_file="/tmp/moatless-vibe-logs-$timestamp.log"
    
    echo -n "Enter number of lines to export (default: 1000): "
    read -r lines
    lines=${lines:-1000}
    
    print_info "Exporting last $lines lines to $output_file..."
    sudo journalctl -u moatless-vibe -n "$lines" --no-pager > "$output_file"
    
    # Also export service info
    echo -e "\n\n=== SERVICE STATUS ===" >> "$output_file"
    sudo systemctl status moatless-vibe --no-pager >> "$output_file"
    
    print_info "Logs exported to: $output_file"
    echo "File size: $(du -h "$output_file" | cut -f1)"
    
    echo -n "Do you want to compress the file? (y/N): "
    read -r compress
    if [[ $compress =~ ^[Yy]$ ]]; then
        gzip "$output_file"
        print_info "Compressed file: ${output_file}.gz"
        echo "Compressed size: $(du -h "${output_file}.gz" | cut -f1)"
    fi
}

# Quick status check
quick_status() {
    print_header "Quick Status Check"
    
    # Service status
    if systemctl is-active --quiet moatless-vibe; then
        echo -e "Service: ${GREEN}● Running${NC}"
    else
        echo -e "Service: ${RED}● Stopped${NC}"
    fi
    
    # Health check
    if curl -f -s "http://localhost:8000/health" > /dev/null; then
        echo -e "Health: ${GREEN}● Healthy${NC}"
    else
        echo -e "Health: ${RED}● Unhealthy${NC}"
    fi
    
    # Recent errors
    ERROR_COUNT=$(sudo journalctl -u moatless-vibe --since "1 hour ago" -p err --no-pager | wc -l)
    if [ "$ERROR_COUNT" -eq 0 ]; then
        echo -e "Recent Errors: ${GREEN}None in last hour${NC}"
    else
        echo -e "Recent Errors: ${YELLOW}$ERROR_COUNT in last hour${NC}"
    fi
    
    # Memory usage
    MEMORY=$(systemctl show moatless-vibe -p MemoryCurrent --value 2>/dev/null)
    if [ -n "$MEMORY" ] && [ "$MEMORY" != "[not set]" ]; then
        echo "Memory Usage: $(echo "$MEMORY" | numfmt --to=iec-i --suffix=B)"
    fi
    
    echo
}

# Main execution
main() {
    # Check if running with proper permissions
    if [ "$EUID" -ne 0 ] && ! groups | grep -q sudo; then
        print_info "Note: Some commands may require sudo permissions"
    fi
    
    # Quick status if no arguments
    if [ $# -eq 0 ]; then
        quick_status
    fi
    
    while true; do
        show_menu
        echo -n "Select option: "
        read -r option
        
        case $option in
            1) view_live_logs ;;
            2) view_last_lines 50 ;;
            3) view_last_lines 100 ;;
            4) view_last_lines 500 ;;
            5) view_time_period "1 hour ago" ;;
            6) view_time_period "today" ;;
            7) view_error_logs ;;
            8) view_warning_error_logs ;;
            9) search_logs ;;
            10) view_service_status ;;
            11) view_docker_logs ;;
            12) export_logs ;;
            0) 
                print_info "Exiting..."
                exit 0 
                ;;
            *)
                echo -e "${RED}Invalid option${NC}"
                ;;
        esac
        
        echo
        echo -n "Press Enter to continue..."
        read -r
    done
}

# Handle command line arguments
case "${1:-}" in
    -f|--follow)
        view_live_logs
        ;;
    -e|--errors)
        view_error_logs
        ;;
    -s|--status)
        view_service_status
        ;;
    -q|--quick)
        quick_status
        ;;
    -h|--help)
        echo "Usage: $0 [OPTIONS]"
        echo "Options:"
        echo "  -f, --follow    View live logs"
        echo "  -e, --errors    View error logs only"
        echo "  -s, --status    View service status"
        echo "  -q, --quick     Quick status check"
        echo "  -h, --help      Show this help"
        echo
        echo "Without options, shows interactive menu"
        ;;
    *)
        main "$@"
        ;;
esac