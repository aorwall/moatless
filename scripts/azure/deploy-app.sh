#!/bin/bash

# Application Deployment Script for Moatless Vibe
# This script deploys the application on the Azure VM

set -e

# Configuration
APP_DIR="/opt/moatless-vibe"
DATA_DIR="/var/lib/moatless-vibe/data"
LOG_DIR="/var/log/moatless-vibe"
SERVICE_FILE="/etc/systemd/system/moatless-vibe.service"
ENV_FILE="$APP_DIR/.env"
REPO_URL="${REPO_URL:-git@github.com:aorwall/moatless-vibe.git}"  # Use SSH by default
BRANCH="${BRANCH:-main}"

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

print_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Check user permissions
check_user() {
    # We'll run as the current user (azureuser) and use sudo where needed
    print_info "Running as user: $USER"
    
    # Ensure we have sudo access
    if ! sudo -n true 2>/dev/null; then
        print_warning "This script requires sudo privileges for some operations"
    fi
}

# Clone or update repository
deploy_code() {
    print_step "Deploying application code..."
    
    cd /opt
    
    # Check if repository already exists
    if [ -d "$APP_DIR/.git" ]; then
        print_info "Repository exists, pulling latest changes..."
        cd "$APP_DIR"
        
        # Add safe.directory config to avoid ownership issues
        git config --global --add safe.directory "$APP_DIR"
        
        # Check current remote URL
        CURRENT_REMOTE=$(git remote get-url origin 2>/dev/null || echo "")
        
        # Update remote if needed (in case we switched from HTTPS to SSH)
        if [ "$CURRENT_REMOTE" != "$REPO_URL" ]; then
            print_info "Updating remote URL to: $REPO_URL"
            git remote set-url origin "$REPO_URL"
        fi
        
        # Fetch and pull latest changes
        git fetch origin
        git checkout "$BRANCH"
        
        # Check if we have uncommitted changes
        if [ -n "$(git status --porcelain)" ]; then
            print_warning "Uncommitted changes detected, stashing them..."
            git stash push -m "Auto-stash before deployment $(date +%Y%m%d_%H%M%S)"
        fi
        
        git pull origin "$BRANCH"
    else
        # Check if directory exists but is not a git repo
        if [ -d "$APP_DIR" ]; then
            print_warning "Directory exists but is not a git repository"
            print_info "Backing up existing directory..."
            sudo mv "$APP_DIR" "$APP_DIR.backup.$(date +%Y%m%d_%H%M%S)"
        fi
        
        print_info "Cloning repository..."
        
        # Ensure parent directory exists
        sudo mkdir -p "$(dirname "$APP_DIR")"
        
        # Clone repository
        git clone -b "$BRANCH" "$REPO_URL" "$APP_DIR"
        cd "$APP_DIR"
    fi
    
    print_info "Current commit: $(git rev-parse --short HEAD)"
    print_info "Current branch: $(git rev-parse --abbrev-ref HEAD)"
}

# Install dependencies
install_dependencies() {
    print_step "Installing application dependencies..."
    
    cd "$APP_DIR"
    
    # Ensure Bun is available
    if ! command -v bun &> /dev/null; then
        export BUN_INSTALL="$HOME/.bun"
        export PATH="$BUN_INSTALL/bin:$PATH"
    fi
    
    # Install dependencies
    print_info "Running bun install..."
    bun install
}

# Build application
build_application() {
    print_step "Building application..."
    
    cd "$APP_DIR"
    
    # Build the application with production API URL
    print_info "Running bun run build..."
    # In production, frontend and backend are on same host/port
    VITE_API_URL="" bun run build
    
    print_info "Build completed successfully"
}

# Setup environment variables
setup_environment() {
    print_step "Setting up environment variables..."
    
    # Check if .env.production exists in the repo
    if [ -f "$APP_DIR/scripts/azure/.env.production" ]; then
        print_info "Using .env.production template from repository"
        sudo cp "$APP_DIR/scripts/azure/.env.production" "$ENV_FILE"
    else
        print_info "Creating default .env file"
        sudo tee "$ENV_FILE" > /dev/null <<EOF
# Moatless Vibe Production Configuration
NODE_ENV=production
PORT=8000
HOST=0.0.0.0

# Data directory for persistent storage
DATA_DIR=$DATA_DIR

# CORS settings (update with your domain)
CORS_ORIGIN=*

# Logging
LOG_LEVEL=info
USE_CONSOLE_LOGGING=false

# Optional: Worker mode configuration
# WORKER_MODE=false
# WORKER_API_URL=
# WORKER_API_KEY=
# WORKSPACE_DIR=

# Optional: Static client path (defaults to ../frontend/dist)
# STATIC_CLIENT_PATH=/opt/moatless-vibe/apps/frontend/dist
EOF
    fi
    
    # Set proper permissions (readable by the service user)
    sudo chmod 644 "$ENV_FILE"
    
    print_warning "Please review and update $ENV_FILE with your specific configuration"
}

# Setup systemd service
setup_service() {
    print_step "Setting up systemd service..."
    
    # Check if service file exists in the repo
    if [ -f "$APP_DIR/scripts/azure/moatless-vibe.service" ]; then
        print_info "Using service file from repository"
        sudo cp "$APP_DIR/scripts/azure/moatless-vibe.service" "$SERVICE_FILE"
    else
        print_info "Creating systemd service file"
        sudo tee "$SERVICE_FILE" > /dev/null <<EOF
[Unit]
Description=Moatless Vibe Application
Documentation=https://github.com/aorwall/moatless-vibe
After=network.target docker.service
Wants=docker.service

[Service]
Type=simple
User=azureuser
Group=azureuser
WorkingDirectory=$APP_DIR
EnvironmentFile=$ENV_FILE
Environment="BUN_INSTALL=/home/azureuser/.bun"
Environment="PATH=/home/azureuser/.bun/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# Start command
ExecStart=/usr/bin/node $APP_DIR/apps/backend/dist/index.js

# Restart policy
Restart=always
RestartSec=10

# Process management
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=30

# Security settings
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$DATA_DIR $LOG_DIR

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=moatless-vibe

[Install]
WantedBy=multi-user.target
EOF
    fi
    
    # Reload systemd
    sudo systemctl daemon-reload
}

# Start or restart service
manage_service() {
    print_step "Managing application service..."
    
    # Check if service is already running
    if systemctl is-active --quiet moatless-vibe; then
        print_info "Service is running, restarting..."
        sudo systemctl restart moatless-vibe
    else
        print_info "Starting service..."
        sudo systemctl start moatless-vibe
    fi
    
    # Enable service to start on boot
    sudo systemctl enable moatless-vibe
    
    # Wait a moment for service to start
    sleep 3
    
    # Check service status
    if systemctl is-active --quiet moatless-vibe; then
        print_info "Service is running successfully"
        sudo systemctl status moatless-vibe --no-pager | head -n 10
    else
        print_error "Service failed to start"
        sudo journalctl -u moatless-vibe -n 50 --no-pager
        exit 1
    fi
}

# Test application
test_application() {
    print_step "Testing application..."
    
    # Get the public IP
    PUBLIC_IP=$(curl -s ifconfig.me)
    
    # Test health endpoint
    print_info "Testing health endpoint..."
    if curl -f -s "http://localhost:8000/health" > /dev/null; then
        print_info "Health check passed!"
        
        # Get health status
        HEALTH=$(curl -s "http://localhost:8000/health")
        echo "Health status: $HEALTH"
    else
        print_error "Health check failed!"
        exit 1
    fi
    
    print_info "Application is accessible at:"
    echo "  - http://$PUBLIC_IP:8000"
    echo "  - http://localhost:8000 (from VM)"
}

# Show logs
show_logs() {
    print_step "Recent application logs:"
    sudo journalctl -u moatless-vibe -n 20 --no-pager
}

# Main execution
main() {
    echo "========================================="
    echo "Moatless Vibe Application Deployment"
    echo "========================================="
    echo
    
    # Check user context
    check_user
    
    # Deployment steps
    deploy_code
    install_dependencies
    build_application
    setup_environment
    setup_service
    manage_service
    test_application
    
    echo
    print_info "========================================="
    print_info "DEPLOYMENT COMPLETE!"
    print_info "========================================="
    echo
    echo "Application is running at:"
    PUBLIC_IP=$(curl -s ifconfig.me)
    echo "  http://$PUBLIC_IP:8000"
    echo
    echo "Useful commands:"
    echo "  View logs:        sudo journalctl -u moatless-vibe -f"
    echo "  Restart service:  sudo systemctl restart moatless-vibe"
    echo "  Stop service:     sudo systemctl stop moatless-vibe"
    echo "  Service status:   sudo systemctl status moatless-vibe"
    echo "  Update app:       $APP_DIR/scripts/azure/update-app.sh"
    echo
    print_warning "Don't forget to:"
    echo "  1. Review and update $ENV_FILE with your configuration"
    echo "  2. Configure DNS to point to $PUBLIC_IP"
    echo "  3. Set up SSL/TLS if needed (using certbot)"
    echo
    
    # Show initial logs
    show_logs
}

# Run main function
main "$@"