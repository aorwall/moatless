#!/bin/bash

# Quick Rebuild Script for Moatless Vibe
# This script pulls latest code, rebuilds with VITE_API_URL from .env, and restarts

set -e

# Configuration
APP_DIR="/opt/moatless-vibe"
ENV_FILE="$APP_DIR/.env"

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

# Main execution
main() {
    print_info "========================================="
    print_info "Quick Rebuild for Moatless Vibe"
    print_info "========================================="
    echo
    
    # Change to app directory
    cd "$APP_DIR"
    
    # Step 1: Git pull
    print_step "Pulling latest code..."
    git pull origin main
    
    # Step 2: Get VITE_API_URL from .env if it exists
    VITE_API_URL=""
    if [ -f "$ENV_FILE" ]; then
        # Extract VITE_API_URL from .env file
        VITE_API_URL=$(grep "^VITE_API_URL=" "$ENV_FILE" | cut -d'=' -f2 | tr -d '"' | tr -d "'" || echo "")
        if [ -n "$VITE_API_URL" ]; then
            print_info "Using VITE_API_URL from .env: $VITE_API_URL"
        else
            print_info "No VITE_API_URL in .env, using empty (relative URLs)"
        fi
    else
        print_warning ".env file not found, using empty VITE_API_URL (relative URLs)"
    fi
    
    # Step 3: Install dependencies if package.json changed
    print_step "Checking dependencies..."
    if git diff HEAD@{1} --name-only | grep -q "package.json\|bun.lockb"; then
        print_info "Package files changed, installing dependencies..."
        bun install
    else
        print_info "No dependency changes"
    fi
    
    # Step 4: Build with VITE_API_URL
    print_step "Building application..."
    if [ -n "$VITE_API_URL" ]; then
        VITE_API_URL="$VITE_API_URL" bun run build
    else
        VITE_API_URL="" bun run build
    fi
    
    # Step 5: Restart service
    print_step "Restarting service..."
    sudo systemctl restart moatless-vibe
    
    # Wait for service to start
    sleep 3
    
    # Step 6: Check status
    if systemctl is-active --quiet moatless-vibe; then
        print_info "Service restarted successfully!"
        
        # Show brief status
        echo
        sudo systemctl status moatless-vibe --no-pager | head -5
        
        # Show access info
        PUBLIC_IP=$(curl -s ifconfig.me 2>/dev/null || echo "unknown")
        echo
        print_info "Application is running at:"
        echo "  Local: http://localhost:8000"
        if [ "$PUBLIC_IP" != "unknown" ]; then
            echo "  Public: http://$PUBLIC_IP:8000"
        fi
        
        # Test health endpoint
        echo
        print_step "Testing health endpoint..."
        if curl -f -s "http://localhost:8000/health" > /dev/null; then
            print_info "Health check passed! ✓"
        else
            print_warning "Health check failed - service may still be starting"
        fi
    else
        print_error "Service failed to start!"
        echo
        print_error "Recent logs:"
        sudo journalctl -u moatless-vibe -n 20 --no-pager
        exit 1
    fi
    
    echo
    print_info "========================================="
    print_info "Rebuild complete!"
    print_info "========================================="
    
    # Show commands for debugging if needed
    echo
    echo "Useful commands:"
    echo "  View logs:  sudo journalctl -u moatless-vibe -f"
    echo "  Status:     sudo systemctl status moatless-vibe"
    echo "  Restart:    sudo systemctl restart moatless-vibe"
}

# Run main function
main "$@"