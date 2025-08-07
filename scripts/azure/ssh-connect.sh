#!/bin/bash

# SSH Connection Helper for Moatless Vibe Azure VM
# This script simplifies SSH connections to the Azure VM

set -e

# Configuration file path
CONFIG_FILE="connection-info.txt"
SSH_KEY_PATH="$HOME/.ssh/id_rsa"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Functions
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Load connection info
load_connection_info() {
    if [ -f "$CONFIG_FILE" ]; then
        print_info "Loading connection info from $CONFIG_FILE"
        source "$CONFIG_FILE"
    else
        print_warning "Connection info file not found. Please provide connection details."
        
        # Try to get from Azure CLI
        if command -v az &> /dev/null; then
            print_info "Attempting to retrieve connection info from Azure..."
            
            RESOURCE_GROUP="${RESOURCE_GROUP:-rg-amplifier}"
            VM_NAME="${VM_NAME:-moatless-vibe-vm}"
            
            # Get public IP
            PUBLIC_IP=$(az vm show -d -g "$RESOURCE_GROUP" -n "$VM_NAME" --query publicIps -o tsv 2>/dev/null)
            
            if [ -n "$PUBLIC_IP" ]; then
                SSH_USER="${SSH_USER:-azureuser}"
                print_info "Found VM: $VM_NAME at $PUBLIC_IP"
            else
                print_error "Could not retrieve VM information from Azure"
                manual_input
            fi
        else
            manual_input
        fi
    fi
}

# Manual input for connection details
manual_input() {
    echo "Please enter connection details:"
    read -p "VM IP address or hostname: " PUBLIC_IP
    read -p "SSH username (default: azureuser): " SSH_USER
    SSH_USER="${SSH_USER:-azureuser}"
    
    # Save for future use
    cat > "$CONFIG_FILE" <<EOF
PUBLIC_IP=$PUBLIC_IP
SSH_USER=$SSH_USER
EOF
    print_info "Connection info saved to $CONFIG_FILE"
}

# SSH connection options
ssh_connect() {
    local mode=$1
    
    # Common SSH options
    SSH_OPTIONS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
    
    # Check if SSH key exists
    if [ ! -f "$SSH_KEY_PATH" ]; then
        print_warning "SSH key not found at $SSH_KEY_PATH"
        SSH_OPTIONS=""
    else
        SSH_OPTIONS="$SSH_OPTIONS -i $SSH_KEY_PATH"
    fi
    
    case $mode in
        "interactive")
            print_info "Connecting to $SSH_USER@$PUBLIC_IP..."
            ssh $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP"
            ;;
        "command")
            shift
            local command="$*"
            print_info "Executing command on remote server..."
            ssh $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP" "$command"
            ;;
        "tunnel")
            local local_port="${2:-8000}"
            local remote_port="${3:-8000}"
            print_info "Creating SSH tunnel from localhost:$local_port to remote:$remote_port"
            print_info "Access the application at: http://localhost:$local_port"
            ssh $SSH_OPTIONS -L "$local_port:localhost:$remote_port" "$SSH_USER@$PUBLIC_IP"
            ;;
        "copy-to")
            local source="$2"
            local dest="${3:-~}"
            print_info "Copying $source to remote:$dest"
            scp $SSH_OPTIONS "$source" "$SSH_USER@$PUBLIC_IP:$dest"
            ;;
        "copy-from")
            local source="$2"
            local dest="${3:-.}"
            print_info "Copying remote:$source to $dest"
            scp $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP:$source" "$dest"
            ;;
        "logs")
            print_info "Viewing application logs..."
            ssh $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP" "sudo journalctl -u moatless-vibe -f"
            ;;
        "status")
            print_info "Checking application status..."
            ssh $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP" "sudo systemctl status moatless-vibe --no-pager && curl -s http://localhost:8000/health | jq"
            ;;
        "restart")
            print_info "Restarting application..."
            ssh $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP" "sudo systemctl restart moatless-vibe && sleep 2 && sudo systemctl status moatless-vibe --no-pager"
            ;;
        "update")
            print_info "Updating application..."
            ssh $SSH_OPTIONS "$SSH_USER@$PUBLIC_IP" "cd /opt/moatless-vibe && ./scripts/azure/update-app.sh"
            ;;
        *)
            show_usage
            ;;
    esac
}

# Show usage
show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo
    echo "Options:"
    echo "  (no args)           - Interactive SSH session"
    echo "  command <cmd>       - Execute command on remote server"
    echo "  tunnel [lport] [rport] - Create SSH tunnel (default: 8000:8000)"
    echo "  copy-to <src> [dst] - Copy file to remote server"
    echo "  copy-from <src> [dst] - Copy file from remote server"
    echo "  logs                - View application logs (live)"
    echo "  status              - Check application status"
    echo "  restart             - Restart application"
    echo "  update              - Update application to latest version"
    echo "  info                - Show connection information"
    echo "  reset               - Reset connection configuration"
    echo
    echo "Examples:"
    echo "  $0                           # Connect via SSH"
    echo "  $0 command 'df -h'          # Check disk usage"
    echo "  $0 tunnel                    # Create tunnel to access app locally"
    echo "  $0 tunnel 3000 8000          # Tunnel local:3000 to remote:8000"
    echo "  $0 copy-to myfile.txt        # Copy file to remote home"
    echo "  $0 logs                      # View live logs"
    echo "  $0 status                    # Check app status"
}

# Show connection info
show_info() {
    print_info "Connection Information:"
    echo "  VM IP: ${PUBLIC_IP:-Not set}"
    echo "  SSH User: ${SSH_USER:-Not set}"
    echo "  SSH Command: ssh ${SSH_USER}@${PUBLIC_IP}"
    echo "  App URL: http://${PUBLIC_IP}:8000"
    
    if [ -f "$SSH_KEY_PATH" ]; then
        echo "  SSH Key: $SSH_KEY_PATH (exists)"
    else
        echo "  SSH Key: $SSH_KEY_PATH (not found)"
    fi
    
    if [ -f "$CONFIG_FILE" ]; then
        echo "  Config File: $CONFIG_FILE"
    fi
}

# Reset configuration
reset_config() {
    print_warning "Resetting connection configuration..."
    rm -f "$CONFIG_FILE"
    print_info "Configuration reset. Run the script again to set up new connection."
}

# Main execution
main() {
    case "${1:-}" in
        "info")
            load_connection_info
            show_info
            ;;
        "reset")
            reset_config
            ;;
        "help"|"-h"|"--help")
            show_usage
            ;;
        *)
            load_connection_info
            ssh_connect "${1:-interactive}" "$@"
            ;;
    esac
}

# Run main function
main "$@"