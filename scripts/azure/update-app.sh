#!/bin/bash

# Update Script for Moatless Vibe
# This script updates the application to the latest version

set -e

# Configuration
APP_DIR="/opt/moatless-vibe"
BRANCH="${BRANCH:-main}"
BACKUP_DIR="/var/backups/moatless-vibe"

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

# Create backup
create_backup() {
    print_step "Creating backup..."
    
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    BACKUP_PATH="$BACKUP_DIR/backup_$TIMESTAMP"
    
    # Create backup directory
    sudo mkdir -p "$BACKUP_DIR"
    
    # Backup current code
    if [ -d "$APP_DIR" ]; then
        sudo cp -r "$APP_DIR" "$BACKUP_PATH"
        print_info "Backup created at: $BACKUP_PATH"
        
        # Keep only last 5 backups
        cd "$BACKUP_DIR"
        ls -t | tail -n +6 | xargs -r sudo rm -rf
    fi
}

# Update code
update_code() {
    print_step "Updating application code..."
    
    cd "$APP_DIR"
    
    # Save current commit hash
    OLD_COMMIT=$(git rev-parse --short HEAD)
    
    # Check if we have uncommitted changes
    if [ -n "$(git status --porcelain)" ]; then
        print_warning "Uncommitted changes detected, stashing them..."
        sudo -u moatless git stash push -m "Auto-stash before update $(date +%Y%m%d_%H%M%S)"
    fi
    
    # Fetch and pull latest changes
    sudo -u moatless git fetch origin
    sudo -u moatless git checkout "$BRANCH"
    sudo -u moatless git pull origin "$BRANCH"
    
    # Get new commit hash
    NEW_COMMIT=$(git rev-parse --short HEAD)
    
    if [ "$OLD_COMMIT" == "$NEW_COMMIT" ]; then
        print_info "Already up to date (commit: $NEW_COMMIT)"
        return 1
    else
        print_info "Updated from $OLD_COMMIT to $NEW_COMMIT"
        
        # Show what changed
        print_info "Changes:"
        git log --oneline "$OLD_COMMIT..$NEW_COMMIT" | head -10
        return 0
    fi
}

# Update dependencies
update_dependencies() {
    print_step "Updating dependencies..."
    
    cd "$APP_DIR"
    
    # Ensure Bun is available
    export BUN_INSTALL="/home/moatless/.bun"
    export PATH="$BUN_INSTALL/bin:$PATH"
    
    # Clean install dependencies
    print_info "Running bun install..."
    sudo -u moatless bash -c "export BUN_INSTALL=/home/moatless/.bun && export PATH=\$BUN_INSTALL/bin:\$PATH && bun install"
}

# Rebuild application
rebuild_application() {
    print_step "Rebuilding application..."
    
    cd "$APP_DIR"
    
    # Build the application
    print_info "Running bun run build..."
    sudo -u moatless bash -c "export BUN_INSTALL=/home/moatless/.bun && export PATH=\$BUN_INSTALL/bin:\$PATH && bun run build"
    
    print_info "Build completed successfully"
}

# Restart service
restart_service() {
    print_step "Restarting service..."
    
    # Restart the service
    sudo systemctl restart moatless-vibe
    
    # Wait for service to start
    sleep 3
    
    # Check service status
    if systemctl is-active --quiet moatless-vibe; then
        print_info "Service restarted successfully"
    else
        print_error "Service failed to restart"
        sudo journalctl -u moatless-vibe -n 50 --no-pager
        
        # Try to restore from backup
        restore_backup
        exit 1
    fi
}

# Test application
test_application() {
    print_step "Testing application..."
    
    # Test health endpoint
    if curl -f -s "http://localhost:8000/health" > /dev/null; then
        print_info "Health check passed!"
        
        # Get health status
        HEALTH=$(curl -s "http://localhost:8000/health" | jq -r '.status')
        print_info "Application status: $HEALTH"
    else
        print_error "Health check failed!"
        
        # Try to restore from backup
        restore_backup
        exit 1
    fi
}

# Restore from backup
restore_backup() {
    print_error "Update failed, attempting to restore from backup..."
    
    # Find latest backup
    LATEST_BACKUP=$(ls -t "$BACKUP_DIR" | head -1)
    
    if [ -n "$LATEST_BACKUP" ]; then
        print_info "Restoring from: $BACKUP_DIR/$LATEST_BACKUP"
        
        # Stop service
        sudo systemctl stop moatless-vibe
        
        # Restore backup
        sudo rm -rf "$APP_DIR"
        sudo cp -r "$BACKUP_DIR/$LATEST_BACKUP" "$APP_DIR"
        
        # Restart service
        sudo systemctl start moatless-vibe
        
        if systemctl is-active --quiet moatless-vibe; then
            print_info "Successfully restored from backup"
        else
            print_error "Failed to restore from backup"
            exit 1
        fi
    else
        print_error "No backup available to restore"
        exit 1
    fi
}

# Show update summary
show_summary() {
    print_step "Update Summary"
    
    echo "Current version:"
    cd "$APP_DIR"
    echo "  Branch: $(git rev-parse --abbrev-ref HEAD)"
    echo "  Commit: $(git rev-parse --short HEAD)"
    echo "  Date: $(git log -1 --format=%cd --date=short)"
    echo "  Message: $(git log -1 --format=%s)"
    echo
    echo "Service status:"
    sudo systemctl status moatless-vibe --no-pager | head -5
    echo
    echo "Recent logs:"
    sudo journalctl -u moatless-vibe -n 10 --no-pager
}

# Main execution
main() {
    echo "========================================="
    echo "Moatless Vibe Application Update"
    echo "========================================="
    echo
    
    # Check if running with proper permissions
    if [ "$EUID" -ne 0 ] && [ "$USER" != "moatless" ]; then
        print_error "Please run with sudo or as moatless user"
        exit 1
    fi
    
    # Create backup before update
    create_backup
    
    # Update the application
    if update_code; then
        update_dependencies
        rebuild_application
        restart_service
        test_application
        
        print_info "========================================="
        print_info "UPDATE COMPLETE!"
        print_info "========================================="
    else
        print_info "No updates available"
    fi
    
    # Show summary
    show_summary
}

# Run main function
main "$@"