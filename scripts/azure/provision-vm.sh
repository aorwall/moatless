#!/bin/bash

# Azure VM Provisioning Script for Moatless Vibe
# This script creates and configures an Azure VM for running the application

set -e

# Configuration
RESOURCE_GROUP="rg-amplifier"
VM_NAME="${VM_NAME:-moatless-vibe-vm}"
LOCATION="${LOCATION:-eastus}"
VM_SIZE="${VM_SIZE:-Standard_B4ms}"  # B4ms has 4 vCPUs, 16GB RAM - good for Docker operations
IMAGE="Canonical:0001-com-ubuntu-server-jammy:22_04-lts-gen2:latest"
ADMIN_USER="${ADMIN_USER:-azureuser}"
DNS_NAME="${DNS_NAME:-moatless-vibe}"
NSG_NAME="${VM_NAME}-nsg"
PUBLIC_IP_NAME="${VM_NAME}-ip"
VNET_NAME="${VM_NAME}-vnet"
SUBNET_NAME="${VM_NAME}-subnet"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
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

check_prerequisites() {
    print_info "Checking prerequisites..."
    
    # Check if Azure CLI is installed
    if ! command -v az &> /dev/null; then
        print_error "Azure CLI is not installed. Please install it first:"
        echo "  Visit: https://docs.microsoft.com/en-us/cli/azure/install-azure-cli"
        exit 1
    fi
    
    # Check if logged in to Azure
    if ! az account show &> /dev/null; then
        print_error "Not logged in to Azure. Please run: az login"
        exit 1
    fi
    
    # Display current subscription
    SUBSCRIPTION=$(az account show --query name -o tsv)
    print_info "Using subscription: $SUBSCRIPTION"
    
    # Check if resource group exists
    if ! az group show --name "$RESOURCE_GROUP" &> /dev/null; then
        print_warning "Resource group '$RESOURCE_GROUP' does not exist."
        read -p "Do you want to create it? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            print_info "Creating resource group '$RESOURCE_GROUP' in location '$LOCATION'..."
            az group create --name "$RESOURCE_GROUP" --location "$LOCATION"
        else
            print_error "Resource group is required. Exiting."
            exit 1
        fi
    else
        print_info "Resource group '$RESOURCE_GROUP' exists."
    fi
}

create_network_resources() {
    print_info "Creating network resources..."
    
    # Create Virtual Network
    print_info "Creating virtual network..."
    az network vnet create \
        --resource-group "$RESOURCE_GROUP" \
        --name "$VNET_NAME" \
        --address-prefix 10.0.0.0/16 \
        --subnet-name "$SUBNET_NAME" \
        --subnet-prefix 10.0.1.0/24
    
    # Create Public IP
    print_info "Creating public IP address..."
    az network public-ip create \
        --resource-group "$RESOURCE_GROUP" \
        --name "$PUBLIC_IP_NAME" \
        --sku Standard \
        --allocation-method Static \
        --dns-name "$DNS_NAME"
    
    # Create Network Security Group
    print_info "Creating network security group..."
    az network nsg create \
        --resource-group "$RESOURCE_GROUP" \
        --name "$NSG_NAME"
    
    # Add NSG rules
    print_info "Adding security rules..."
    
    # SSH
    az network nsg rule create \
        --resource-group "$RESOURCE_GROUP" \
        --nsg-name "$NSG_NAME" \
        --name AllowSSH \
        --priority 1000 \
        --source-address-prefixes "*" \
        --destination-port-ranges 22 \
        --access Allow \
        --protocol Tcp \
        --description "Allow SSH"
    
    # Application port
    az network nsg rule create \
        --resource-group "$RESOURCE_GROUP" \
        --nsg-name "$NSG_NAME" \
        --name AllowApp \
        --priority 1001 \
        --source-address-prefixes "*" \
        --destination-port-ranges 8000 \
        --access Allow \
        --protocol Tcp \
        --description "Allow application traffic"
    
    # HTTP (optional, for future use)
    az network nsg rule create \
        --resource-group "$RESOURCE_GROUP" \
        --nsg-name "$NSG_NAME" \
        --name AllowHTTP \
        --priority 1002 \
        --source-address-prefixes "*" \
        --destination-port-ranges 80 \
        --access Allow \
        --protocol Tcp \
        --description "Allow HTTP"
    
    # HTTPS (optional, for future use)
    az network nsg rule create \
        --resource-group "$RESOURCE_GROUP" \
        --nsg-name "$NSG_NAME" \
        --name AllowHTTPS \
        --priority 1003 \
        --source-address-prefixes "*" \
        --destination-port-ranges 443 \
        --access Allow \
        --protocol Tcp \
        --description "Allow HTTPS"
}

create_vm() {
    print_info "Creating virtual machine..."
    
    # Check for SSH key
    SSH_KEY_PATH="$HOME/.ssh/id_rsa.pub"
    if [ ! -f "$SSH_KEY_PATH" ]; then
        print_warning "SSH public key not found at $SSH_KEY_PATH"
        print_info "Generating SSH key pair..."
        ssh-keygen -t rsa -b 4096 -f "$HOME/.ssh/id_rsa" -N ""
    fi
    
    # Create VM
    az vm create \
        --resource-group "$RESOURCE_GROUP" \
        --name "$VM_NAME" \
        --image "$IMAGE" \
        --size "$VM_SIZE" \
        --admin-username "$ADMIN_USER" \
        --ssh-key-values "$SSH_KEY_PATH" \
        --public-ip-address "$PUBLIC_IP_NAME" \
        --vnet-name "$VNET_NAME" \
        --subnet "$SUBNET_NAME" \
        --nsg "$NSG_NAME" \
        --storage-sku Premium_LRS \
        --os-disk-size-gb 64 \
        --tags "Application=MoatlessVibe" "Environment=Production"
    
    # Enable auto-shutdown (optional - saves costs)
    print_info "Configuring auto-shutdown (daily at 11 PM UTC)..."
    az vm auto-shutdown \
        --resource-group "$RESOURCE_GROUP" \
        --name "$VM_NAME" \
        --time 2300 \
        --email "$ADMIN_USER@example.com" 2>/dev/null || print_warning "Auto-shutdown configuration failed (optional feature)"
}

get_connection_info() {
    print_info "Retrieving connection information..."
    
    # Get public IP
    PUBLIC_IP=$(az network public-ip show \
        --resource-group "$RESOURCE_GROUP" \
        --name "$PUBLIC_IP_NAME" \
        --query ipAddress -o tsv)
    
    # Get FQDN
    FQDN=$(az network public-ip show \
        --resource-group "$RESOURCE_GROUP" \
        --name "$PUBLIC_IP_NAME" \
        --query dnsSettings.fqdn -o tsv)
    
    print_info "========================================="
    print_info "VM PROVISIONING COMPLETE!"
    print_info "========================================="
    echo
    echo "VM Name:       $VM_NAME"
    echo "Resource Group: $RESOURCE_GROUP"
    echo "Location:      $LOCATION"
    echo "VM Size:       $VM_SIZE"
    echo "Public IP:     $PUBLIC_IP"
    echo "FQDN:          $FQDN"
    echo "SSH User:      $ADMIN_USER"
    echo
    echo "To connect via SSH:"
    echo "  ssh $ADMIN_USER@$PUBLIC_IP"
    echo "  or"
    echo "  ssh $ADMIN_USER@$FQDN"
    echo
    echo "Next steps:"
    echo "1. SSH into the VM"
    echo "2. Run: curl -sSL https://raw.githubusercontent.com/[your-repo]/scripts/azure/init-vm.sh | bash"
    echo "3. Run: curl -sSL https://raw.githubusercontent.com/[your-repo]/scripts/azure/deploy-app.sh | bash"
    echo
    echo "Application will be available at:"
    echo "  http://$PUBLIC_IP:8000"
    echo "  http://$FQDN:8000"
    
    # Save connection info to file
    cat > connection-info.txt <<EOF
VM_NAME=$VM_NAME
RESOURCE_GROUP=$RESOURCE_GROUP
PUBLIC_IP=$PUBLIC_IP
FQDN=$FQDN
SSH_USER=$ADMIN_USER
SSH_COMMAND=ssh $ADMIN_USER@$PUBLIC_IP
APP_URL=http://$PUBLIC_IP:8000
EOF
    
    print_info "Connection info saved to: connection-info.txt"
}

# Main execution
main() {
    echo "========================================="
    echo "Azure VM Provisioning for Moatless Vibe"
    echo "========================================="
    echo
    
    check_prerequisites
    
    # Check if VM already exists
    if az vm show --resource-group "$RESOURCE_GROUP" --name "$VM_NAME" &> /dev/null; then
        print_warning "VM '$VM_NAME' already exists in resource group '$RESOURCE_GROUP'"
        read -p "Do you want to delete it and create a new one? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            print_info "Deleting existing VM and associated resources..."
            az vm delete --resource-group "$RESOURCE_GROUP" --name "$VM_NAME" --yes
            az network nsg delete --resource-group "$RESOURCE_GROUP" --name "$NSG_NAME" --yes 2>/dev/null || true
            az network public-ip delete --resource-group "$RESOURCE_GROUP" --name "$PUBLIC_IP_NAME" --yes 2>/dev/null || true
            az network vnet delete --resource-group "$RESOURCE_GROUP" --name "$VNET_NAME" --yes 2>/dev/null || true
        else
            print_info "Exiting without changes."
            exit 0
        fi
    fi
    
    create_network_resources
    create_vm
    get_connection_info
}

# Run main function
main "$@"