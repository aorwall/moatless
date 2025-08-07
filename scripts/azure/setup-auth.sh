#!/bin/bash

# Authentication Setup Script for Moatless Vibe
# This script helps configure basic authentication for the application

set -e

# Configuration
ENV_FILE="/opt/moatless-vibe/.env"

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

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_header() {
    echo -e "${BLUE}=========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}=========================================${NC}"
}

# Generate secure password
generate_password() {
    # Generate a secure random password
    openssl rand -base64 32 | tr -d "=+/" | cut -c1-25
}

# Generate secure token
generate_token() {
    # Generate a secure random token
    openssl rand -hex 32
}

# Setup basic authentication
setup_basic_auth() {
    print_header "Setting up Basic Authentication"
    
    # Check if .env file exists
    if [ ! -f "$ENV_FILE" ]; then
        print_error "Environment file not found at $ENV_FILE"
        print_info "Please run deploy-app.sh first"
        exit 1
    fi
    
    # Backup current .env file
    sudo cp "$ENV_FILE" "$ENV_FILE.backup.$(date +%Y%m%d_%H%M%S)"
    
    # Ask for username
    echo -n "Enter username (default: admin): "
    read -r username
    username="${username:-admin}"
    
    # Ask for password or generate one
    echo "Password options:"
    echo "1) Generate secure password (recommended)"
    echo "2) Enter custom password"
    echo -n "Choose option (1 or 2): "
    read -r password_option
    
    if [ "$password_option" = "2" ]; then
        echo -n "Enter password: "
        read -rs password
        echo
        echo -n "Confirm password: "
        read -rs password_confirm
        echo
        
        if [ "$password" != "$password_confirm" ]; then
            print_error "Passwords do not match"
            exit 1
        fi
        
        if [ ${#password} -lt 8 ]; then
            print_error "Password must be at least 8 characters long"
            exit 1
        fi
    else
        password=$(generate_password)
        print_info "Generated secure password"
    fi
    
    # Update .env file
    print_info "Updating environment configuration..."
    
    # Remove existing auth settings if present
    sudo sed -i '/^BASIC_AUTH_ENABLED=/d' "$ENV_FILE"
    sudo sed -i '/^BASIC_AUTH_USER=/d' "$ENV_FILE"
    sudo sed -i '/^BASIC_AUTH_PASS=/d' "$ENV_FILE"
    
    # Add new auth settings
    echo "" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "# Basic Authentication" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "BASIC_AUTH_ENABLED=true" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "BASIC_AUTH_USER=$username" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "BASIC_AUTH_PASS=$password" | sudo tee -a "$ENV_FILE" > /dev/null
    
    # Set proper permissions
    sudo chown moatless:moatless "$ENV_FILE"
    sudo chmod 600 "$ENV_FILE"
    
    print_info "Basic authentication configured successfully!"
    echo
    print_header "Authentication Credentials"
    echo "Username: $username"
    echo "Password: $password"
    echo
    print_warning "IMPORTANT: Save these credentials securely!"
    print_warning "You will need them to access the application"
    echo
    
    # Ask to restart service
    echo -n "Restart the application now? (y/N): "
    read -r restart
    if [[ $restart =~ ^[Yy]$ ]]; then
        sudo systemctl restart moatless-vibe
        sleep 3
        if systemctl is-active --quiet moatless-vibe; then
            print_info "Application restarted successfully"
            
            # Get public IP
            PUBLIC_IP=$(curl -s ifconfig.me)
            echo
            print_info "Application is now protected with basic authentication"
            echo "Access at: http://$PUBLIC_IP:8000"
            echo "Use the credentials above to log in"
        else
            print_error "Failed to restart application"
            sudo journalctl -u moatless-vibe -n 20 --no-pager
        fi
    else
        print_info "Remember to restart the service for changes to take effect:"
        echo "  sudo systemctl restart moatless-vibe"
    fi
}

# Setup token authentication
setup_token_auth() {
    print_header "Setting up Token Authentication"
    
    # Check if .env file exists
    if [ ! -f "$ENV_FILE" ]; then
        print_error "Environment file not found at $ENV_FILE"
        print_info "Please run deploy-app.sh first"
        exit 1
    fi
    
    # Backup current .env file
    sudo cp "$ENV_FILE" "$ENV_FILE.backup.$(date +%Y%m%d_%H%M%S)"
    
    # Ask for token or generate one
    echo "Token options:"
    echo "1) Generate secure token (recommended)"
    echo "2) Enter custom token"
    echo -n "Choose option (1 or 2): "
    read -r token_option
    
    if [ "$token_option" = "2" ]; then
        echo -n "Enter token: "
        read -r token
        
        if [ ${#token} -lt 20 ]; then
            print_error "Token must be at least 20 characters long"
            exit 1
        fi
    else
        token=$(generate_token)
        print_info "Generated secure token"
    fi
    
    # Update .env file
    print_info "Updating environment configuration..."
    
    # Remove existing auth settings if present
    sudo sed -i '/^AUTH_TOKEN_ENABLED=/d' "$ENV_FILE"
    sudo sed -i '/^AUTH_TOKEN=/d' "$ENV_FILE"
    sudo sed -i '/^BASIC_AUTH_ENABLED=/d' "$ENV_FILE"
    
    # Disable basic auth and enable token auth
    echo "" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "# Token Authentication" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "BASIC_AUTH_ENABLED=false" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "AUTH_TOKEN_ENABLED=true" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "AUTH_TOKEN=$token" | sudo tee -a "$ENV_FILE" > /dev/null
    
    # Set proper permissions
    sudo chown moatless:moatless "$ENV_FILE"
    sudo chmod 600 "$ENV_FILE"
    
    print_info "Token authentication configured successfully!"
    echo
    print_header "Authentication Token"
    echo "$token"
    echo
    print_warning "IMPORTANT: Save this token securely!"
    print_warning "You will need it to access the API"
    echo
    print_info "Usage examples:"
    echo "  Header: Authorization: Bearer $token"
    echo "  Query: ?token=$token"
    echo "  Header: X-Auth-Token: $token"
    echo
    
    # Ask to restart service
    echo -n "Restart the application now? (y/N): "
    read -r restart
    if [[ $restart =~ ^[Yy]$ ]]; then
        sudo systemctl restart moatless-vibe
        sleep 3
        if systemctl is-active --quiet moatless-vibe; then
            print_info "Application restarted successfully"
            
            # Get public IP
            PUBLIC_IP=$(curl -s ifconfig.me)
            echo
            print_info "Application is now protected with token authentication"
            echo "Test with: curl -H 'Authorization: Bearer $token' http://$PUBLIC_IP:8000/health"
        else
            print_error "Failed to restart application"
            sudo journalctl -u moatless-vibe -n 20 --no-pager
        fi
    else
        print_info "Remember to restart the service for changes to take effect:"
        echo "  sudo systemctl restart moatless-vibe"
    fi
}

# Disable authentication
disable_auth() {
    print_header "Disabling Authentication"
    
    print_warning "This will remove all authentication from your application!"
    echo -n "Are you sure? (yes/no): "
    read -r confirm
    
    if [ "$confirm" != "yes" ]; then
        print_info "Cancelled"
        exit 0
    fi
    
    # Check if .env file exists
    if [ ! -f "$ENV_FILE" ]; then
        print_error "Environment file not found at $ENV_FILE"
        exit 1
    fi
    
    # Backup current .env file
    sudo cp "$ENV_FILE" "$ENV_FILE.backup.$(date +%Y%m%d_%H%M%S)"
    
    # Update .env file
    print_info "Updating environment configuration..."
    
    # Remove existing auth settings
    sudo sed -i '/^BASIC_AUTH_ENABLED=/d' "$ENV_FILE"
    sudo sed -i '/^BASIC_AUTH_USER=/d' "$ENV_FILE"
    sudo sed -i '/^BASIC_AUTH_PASS=/d' "$ENV_FILE"
    sudo sed -i '/^AUTH_TOKEN_ENABLED=/d' "$ENV_FILE"
    sudo sed -i '/^AUTH_TOKEN=/d' "$ENV_FILE"
    
    # Add disabled settings
    echo "" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "# Authentication disabled" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "BASIC_AUTH_ENABLED=false" | sudo tee -a "$ENV_FILE" > /dev/null
    echo "AUTH_TOKEN_ENABLED=false" | sudo tee -a "$ENV_FILE" > /dev/null
    
    # Restart service
    print_info "Restarting application..."
    sudo systemctl restart moatless-vibe
    sleep 3
    
    if systemctl is-active --quiet moatless-vibe; then
        print_info "Authentication disabled successfully"
        print_warning "Your application is now publicly accessible!"
    else
        print_error "Failed to restart application"
        sudo journalctl -u moatless-vibe -n 20 --no-pager
    fi
}

# Show current auth status
show_status() {
    print_header "Authentication Status"
    
    if [ ! -f "$ENV_FILE" ]; then
        print_error "Environment file not found at $ENV_FILE"
        exit 1
    fi
    
    # Check basic auth
    BASIC_AUTH=$(grep "^BASIC_AUTH_ENABLED=" "$ENV_FILE" | cut -d'=' -f2)
    BASIC_USER=$(grep "^BASIC_AUTH_USER=" "$ENV_FILE" | cut -d'=' -f2)
    
    # Check token auth
    TOKEN_AUTH=$(grep "^AUTH_TOKEN_ENABLED=" "$ENV_FILE" | cut -d'=' -f2)
    
    echo "Basic Authentication:"
    if [ "$BASIC_AUTH" = "true" ]; then
        echo -e "  Status: ${GREEN}Enabled${NC}"
        echo "  Username: $BASIC_USER"
    else
        echo -e "  Status: ${YELLOW}Disabled${NC}"
    fi
    
    echo
    echo "Token Authentication:"
    if [ "$TOKEN_AUTH" = "true" ]; then
        echo -e "  Status: ${GREEN}Enabled${NC}"
        echo "  Token: [hidden]"
    else
        echo -e "  Status: ${YELLOW}Disabled${NC}"
    fi
    
    echo
    # Get public IP
    PUBLIC_IP=$(curl -s ifconfig.me)
    echo "Application URL: http://$PUBLIC_IP:8000"
    
    # Test health endpoint
    echo
    echo "Testing access..."
    if curl -f -s "http://localhost:8000/health" > /dev/null; then
        echo -e "  Health check: ${GREEN}Accessible${NC}"
    else
        echo -e "  Health check: ${RED}Protected or unavailable${NC}"
    fi
}

# Show menu
show_menu() {
    print_header "Moatless Vibe Authentication Setup"
    echo "1) Setup Basic Authentication (for browser access)"
    echo "2) Setup Token Authentication (for API access)"
    echo "3) Disable Authentication"
    echo "4) Show Current Status"
    echo "0) Exit"
    echo
}

# Main execution
main() {
    # Check if running with proper permissions
    if [ "$EUID" -ne 0 ] && [ "$USER" != "moatless" ]; then
        print_warning "Some operations may require sudo permissions"
    fi
    
    # If no arguments, show menu
    if [ $# -eq 0 ]; then
        while true; do
            show_menu
            echo -n "Select option: "
            read -r option
            
            case $option in
                1) setup_basic_auth; break ;;
                2) setup_token_auth; break ;;
                3) disable_auth; break ;;
                4) show_status; break ;;
                0) 
                    print_info "Exiting..."
                    exit 0 
                    ;;
                *)
                    print_error "Invalid option"
                    ;;
            esac
        done
    else
        # Handle command line arguments
        case "$1" in
            basic)
                setup_basic_auth
                ;;
            token)
                setup_token_auth
                ;;
            disable)
                disable_auth
                ;;
            status)
                show_status
                ;;
            *)
                echo "Usage: $0 [basic|token|disable|status]"
                echo "  basic   - Setup basic authentication"
                echo "  token   - Setup token authentication"
                echo "  disable - Disable authentication"
                echo "  status  - Show current status"
                echo
                echo "Without arguments, shows interactive menu"
                exit 1
                ;;
        esac
    fi
}

# Run main function
main "$@"