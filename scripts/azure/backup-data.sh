#!/bin/bash

# Data Backup Script for Moatless Vibe
# This script backs up application data and configuration

set -e

# Configuration
DATA_DIR="/var/lib/moatless-vibe/data"
APP_DIR="/opt/moatless-vibe"
BACKUP_BASE_DIR="/var/backups/moatless-vibe"
REMOTE_BACKUP_DIR="${REMOTE_BACKUP_DIR:-}"  # Optional: Azure Storage or remote location
MAX_LOCAL_BACKUPS=7  # Keep last 7 backups locally

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

# Check prerequisites
check_prerequisites() {
    # Check if running with proper permissions
    if [ "$EUID" -ne 0 ]; then
        print_error "This script must be run with sudo"
        exit 1
    fi
    
    # Check if required directories exist
    if [ ! -d "$DATA_DIR" ]; then
        print_warning "Data directory does not exist: $DATA_DIR"
    fi
    
    # Create backup directory if it doesn't exist
    mkdir -p "$BACKUP_BASE_DIR"
}

# Create backup
create_backup() {
    print_step "Creating backup..."
    
    # Generate timestamp
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    BACKUP_NAME="backup_${TIMESTAMP}"
    BACKUP_DIR="$BACKUP_BASE_DIR/$BACKUP_NAME"
    
    # Create backup directory
    mkdir -p "$BACKUP_DIR"
    
    # Backup data directory
    if [ -d "$DATA_DIR" ]; then
        print_info "Backing up data directory..."
        tar -czf "$BACKUP_DIR/data.tar.gz" -C "$(dirname "$DATA_DIR")" "$(basename "$DATA_DIR")" 2>/dev/null || {
            print_warning "Data directory is empty or inaccessible"
        }
    fi
    
    # Backup environment configuration
    if [ -f "$APP_DIR/.env" ]; then
        print_info "Backing up environment configuration..."
        cp "$APP_DIR/.env" "$BACKUP_DIR/env.backup"
    fi
    
    # Backup systemd service file
    if [ -f "/etc/systemd/system/moatless-vibe.service" ]; then
        print_info "Backing up service configuration..."
        cp "/etc/systemd/system/moatless-vibe.service" "$BACKUP_DIR/service.backup"
    fi
    
    # Create backup metadata
    cat > "$BACKUP_DIR/metadata.json" <<EOF
{
    "timestamp": "$TIMESTAMP",
    "date": "$(date -Iseconds)",
    "hostname": "$(hostname)",
    "data_size": "$(du -sh "$DATA_DIR" 2>/dev/null | cut -f1)",
    "app_version": "$(cd "$APP_DIR" && git rev-parse --short HEAD 2>/dev/null || echo 'unknown')",
    "app_branch": "$(cd "$APP_DIR" && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')"
}
EOF
    
    # Create compressed archive of entire backup
    print_info "Compressing backup..."
    cd "$BACKUP_BASE_DIR"
    tar -czf "${BACKUP_NAME}.tar.gz" "$BACKUP_NAME"
    
    # Remove uncompressed backup directory
    rm -rf "$BACKUP_DIR"
    
    # Calculate backup size
    BACKUP_SIZE=$(du -h "${BACKUP_BASE_DIR}/${BACKUP_NAME}.tar.gz" | cut -f1)
    print_info "Backup created: ${BACKUP_NAME}.tar.gz (Size: $BACKUP_SIZE)"
    
    echo "$BACKUP_BASE_DIR/${BACKUP_NAME}.tar.gz"
}

# Clean old backups
clean_old_backups() {
    print_step "Cleaning old backups..."
    
    cd "$BACKUP_BASE_DIR"
    
    # Count existing backups
    BACKUP_COUNT=$(ls -1 backup_*.tar.gz 2>/dev/null | wc -l)
    
    if [ "$BACKUP_COUNT" -gt "$MAX_LOCAL_BACKUPS" ]; then
        # Calculate how many to delete
        DELETE_COUNT=$((BACKUP_COUNT - MAX_LOCAL_BACKUPS))
        
        print_info "Removing $DELETE_COUNT old backup(s)..."
        
        # Delete oldest backups
        ls -1t backup_*.tar.gz | tail -n "$DELETE_COUNT" | while read -r old_backup; do
            print_info "Removing: $old_backup"
            rm -f "$old_backup"
        done
    else
        print_info "Current backups: $BACKUP_COUNT (max: $MAX_LOCAL_BACKUPS)"
    fi
}

# Upload to remote storage (Azure Storage)
upload_to_azure() {
    local backup_file=$1
    
    print_step "Uploading to Azure Storage..."
    
    # Check if Azure CLI is available
    if ! command -v az &> /dev/null; then
        print_warning "Azure CLI not installed, skipping remote backup"
        return
    fi
    
    # Check if storage account is configured
    if [ -z "$AZURE_STORAGE_ACCOUNT" ] || [ -z "$AZURE_STORAGE_CONTAINER" ]; then
        print_warning "Azure Storage not configured. Set AZURE_STORAGE_ACCOUNT and AZURE_STORAGE_CONTAINER"
        return
    fi
    
    # Upload to Azure Blob Storage
    BLOB_NAME="$(basename "$backup_file")"
    
    az storage blob upload \
        --account-name "$AZURE_STORAGE_ACCOUNT" \
        --container-name "$AZURE_STORAGE_CONTAINER" \
        --name "$BLOB_NAME" \
        --file "$backup_file" \
        --auth-mode login
    
    if [ $? -eq 0 ]; then
        print_info "Backup uploaded to Azure Storage: $BLOB_NAME"
    else
        print_error "Failed to upload backup to Azure Storage"
    fi
}

# List backups
list_backups() {
    print_step "Available backups:"
    
    echo
    echo "Local backups in $BACKUP_BASE_DIR:"
    ls -lht "$BACKUP_BASE_DIR"/backup_*.tar.gz 2>/dev/null | head -10 || echo "  No backups found"
    
    echo
    echo "Backup disk usage:"
    du -sh "$BACKUP_BASE_DIR" 2>/dev/null
    
    # List Azure backups if configured
    if [ -n "$AZURE_STORAGE_ACCOUNT" ] && [ -n "$AZURE_STORAGE_CONTAINER" ] && command -v az &> /dev/null; then
        echo
        echo "Azure Storage backups:"
        az storage blob list \
            --account-name "$AZURE_STORAGE_ACCOUNT" \
            --container-name "$AZURE_STORAGE_CONTAINER" \
            --auth-mode login \
            --query "[?contains(name, 'backup_')].{name:name, size:properties.contentLength, modified:properties.lastModified}" \
            --output table 2>/dev/null || echo "  Unable to list Azure backups"
    fi
}

# Restore from backup
restore_backup() {
    local backup_file=$1
    
    if [ -z "$backup_file" ]; then
        print_error "Please specify a backup file to restore"
        list_backups
        exit 1
    fi
    
    if [ ! -f "$backup_file" ]; then
        print_error "Backup file not found: $backup_file"
        exit 1
    fi
    
    print_warning "This will restore data from: $backup_file"
    print_warning "Current data will be overwritten!"
    echo -n "Are you sure? (yes/no): "
    read -r confirmation
    
    if [ "$confirmation" != "yes" ]; then
        print_info "Restore cancelled"
        exit 0
    fi
    
    print_step "Restoring from backup..."
    
    # Create temporary directory
    TEMP_DIR=$(mktemp -d)
    
    # Extract backup
    print_info "Extracting backup..."
    tar -xzf "$backup_file" -C "$TEMP_DIR"
    
    # Find the backup directory
    BACKUP_CONTENT=$(ls -1 "$TEMP_DIR" | head -1)
    RESTORE_DIR="$TEMP_DIR/$BACKUP_CONTENT"
    
    # Stop the service
    print_info "Stopping service..."
    systemctl stop moatless-vibe
    
    # Backup current data (just in case)
    if [ -d "$DATA_DIR" ]; then
        print_info "Creating safety backup of current data..."
        mv "$DATA_DIR" "${DATA_DIR}.before-restore.$(date +%Y%m%d_%H%M%S)"
    fi
    
    # Restore data
    if [ -f "$RESTORE_DIR/data.tar.gz" ]; then
        print_info "Restoring data..."
        tar -xzf "$RESTORE_DIR/data.tar.gz" -C "$(dirname "$DATA_DIR")"
    fi
    
    # Restore environment configuration
    if [ -f "$RESTORE_DIR/env.backup" ]; then
        print_info "Restoring environment configuration..."
        cp "$RESTORE_DIR/env.backup" "$APP_DIR/.env"
        chown moatless:moatless "$APP_DIR/.env"
    fi
    
    # Restore service configuration
    if [ -f "$RESTORE_DIR/service.backup" ]; then
        print_info "Restoring service configuration..."
        cp "$RESTORE_DIR/service.backup" "/etc/systemd/system/moatless-vibe.service"
        systemctl daemon-reload
    fi
    
    # Clean up
    rm -rf "$TEMP_DIR"
    
    # Start the service
    print_info "Starting service..."
    systemctl start moatless-vibe
    
    # Check service status
    sleep 3
    if systemctl is-active --quiet moatless-vibe; then
        print_info "Restore completed successfully!"
        systemctl status moatless-vibe --no-pager | head -5
    else
        print_error "Service failed to start after restore"
        journalctl -u moatless-vibe -n 20 --no-pager
    fi
}

# Automated backup (for cron)
automated_backup() {
    print_info "Starting automated backup..."
    
    # Create backup
    BACKUP_FILE=$(create_backup)
    
    # Clean old backups
    clean_old_backups
    
    # Upload to remote if configured
    if [ -n "$REMOTE_BACKUP_DIR" ] || [ -n "$AZURE_STORAGE_ACCOUNT" ]; then
        upload_to_azure "$BACKUP_FILE"
    fi
    
    print_info "Automated backup completed"
}

# Setup cron job
setup_cron() {
    print_step "Setting up automated daily backup..."
    
    CRON_FILE="/etc/cron.d/moatless-vibe-backup"
    
    cat > "$CRON_FILE" <<EOF
# Moatless Vibe Daily Backup
# Runs every day at 2:00 AM
0 2 * * * root /opt/moatless-vibe/scripts/azure/backup-data.sh auto >> /var/log/moatless-vibe/backup.log 2>&1
EOF
    
    chmod 644 "$CRON_FILE"
    print_info "Cron job created: Daily backup at 2:00 AM"
    print_info "Log file: /var/log/moatless-vibe/backup.log"
}

# Show usage
show_usage() {
    echo "Usage: $0 [COMMAND] [OPTIONS]"
    echo
    echo "Commands:"
    echo "  create              - Create a new backup"
    echo "  list                - List available backups"
    echo "  restore <file>      - Restore from a backup file"
    echo "  clean               - Remove old backups"
    echo "  auto                - Automated backup (for cron)"
    echo "  setup-cron          - Setup daily automated backup"
    echo
    echo "Environment variables:"
    echo "  AZURE_STORAGE_ACCOUNT   - Azure Storage account name"
    echo "  AZURE_STORAGE_CONTAINER - Azure Storage container name"
    echo "  MAX_LOCAL_BACKUPS       - Number of backups to keep (default: 7)"
    echo
    echo "Examples:"
    echo "  $0 create                    # Create a backup"
    echo "  $0 list                      # List all backups"
    echo "  $0 restore backup_file.tar.gz # Restore from backup"
    echo "  $0 setup-cron                # Setup daily backups"
}

# Main execution
main() {
    case "${1:-create}" in
        create)
            check_prerequisites
            BACKUP_FILE=$(create_backup)
            clean_old_backups
            print_info "Backup complete: $BACKUP_FILE"
            ;;
        list)
            list_backups
            ;;
        restore)
            check_prerequisites
            restore_backup "$2"
            ;;
        clean)
            check_prerequisites
            clean_old_backups
            ;;
        auto)
            check_prerequisites
            automated_backup
            ;;
        setup-cron)
            check_prerequisites
            setup_cron
            ;;
        help|--help|-h)
            show_usage
            ;;
        *)
            print_error "Unknown command: $1"
            show_usage
            exit 1
            ;;
    esac
}

# Run main function
main "$@"