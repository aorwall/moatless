#!/bin/bash

# Azure VM Initialization Script for Moatless Vibe
# This script installs all necessary dependencies on the VM

set -e

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

# Update system
update_system() {
    print_info "Updating system packages..."
    sudo apt-get update
    sudo apt-get upgrade -y
    sudo apt-get install -y curl wget git build-essential unzip
}

# Install Node.js 22.x
install_nodejs() {
    print_info "Installing Node.js 22.x..."
    
    # Remove any existing Node.js installations first
    sudo apt-get remove -y nodejs npm 2>/dev/null || true
    
    # Install required packages for NodeSource repository
    sudo apt-get install -y ca-certificates curl gnupg
    
    # Create keyrings directory
    sudo mkdir -p /etc/apt/keyrings
    
    # Add NodeSource GPG key
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | sudo gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg
    
    # Add NodeSource repository
    NODE_MAJOR=22
    echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_$NODE_MAJOR.x nodistro main" | sudo tee /etc/apt/sources.list.d/nodesource.list
    
    # Update package list
    sudo apt-get update
    
    # Install Node.js (includes npm)
    sudo apt-get install -y nodejs
    
    # Verify installation
    NODE_VERSION=$(node --version 2>/dev/null || echo "not installed")
    NPM_VERSION=$(npm --version 2>/dev/null || echo "not installed")
    print_info "Node.js installed: $NODE_VERSION"
    print_info "npm installed: $NPM_VERSION"
    
    # Check if installation succeeded
    if [ "$NODE_VERSION" = "not installed" ]; then
        print_error "Node.js installation failed!"
        exit 1
    fi
}

# Install Bun
install_bun() {
    print_info "Installing Bun package manager..."
    
    # Install Bun
    curl -fsSL https://bun.sh/install | bash
    
    # Add to PATH for current session
    export BUN_INSTALL="$HOME/.bun"
    export PATH="$BUN_INSTALL/bin:$PATH"
    
    # Add to bashrc for future sessions
    echo 'export BUN_INSTALL="$HOME/.bun"' >> ~/.bashrc
    echo 'export PATH="$BUN_INSTALL/bin:$PATH"' >> ~/.bashrc
    
    # Verify installation
    source ~/.bashrc
    BUN_VERSION=$($HOME/.bun/bin/bun --version)
    print_info "Bun installed: $BUN_VERSION"
}

# Install Java and Maven
install_java_maven() {
    print_info "Installing Java 17 and Maven..."
    
    # Install OpenJDK 17
    sudo apt-get install -y openjdk-17-jdk openjdk-17-jre
    
    # Install Maven
    sudo apt-get install -y maven
    
    # Set JAVA_HOME
    echo 'export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64' | sudo tee -a /etc/environment
    echo 'export PATH=$PATH:$JAVA_HOME/bin' | sudo tee -a /etc/environment
    source /etc/environment
    
    # Verify installation
    JAVA_VERSION=$(java -version 2>&1 | head -n 1)
    MVN_VERSION=$(mvn -version | head -n 1)
    print_info "Java installed: $JAVA_VERSION"
    print_info "Maven installed: $MVN_VERSION"
}

# Install Docker
install_docker() {
    print_info "Installing Docker..."
    
    # Remove old versions
    sudo apt-get remove -y docker docker-engine docker.io containerd runc 2>/dev/null || true
    
    # Install prerequisites
    sudo apt-get install -y \
        apt-transport-https \
        ca-certificates \
        curl \
        gnupg \
        lsb-release
    
    # Add Docker's official GPG key
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg
    
    # Set up the stable repository
    echo \
        "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu \
        $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
    
    # Install Docker Engine
    sudo apt-get update
    sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
    
    # Add current user to docker group
    sudo usermod -aG docker $USER
    
    # Start and enable Docker
    sudo systemctl start docker
    sudo systemctl enable docker
    
    # Verify installation
    DOCKER_VERSION=$(sudo docker version --format '{{.Server.Version}}')
    print_info "Docker installed: $DOCKER_VERSION"
    
    print_warning "You need to log out and back in for docker group membership to take effect"
}

# Configure firewall
configure_firewall() {
    print_info "Configuring firewall..."
    
    # Install UFW if not already installed
    sudo apt-get install -y ufw
    
    # Set default policies
    sudo ufw default deny incoming
    sudo ufw default allow outgoing
    
    # Allow SSH
    sudo ufw allow 22/tcp comment 'SSH'
    
    # Allow application port
    sudo ufw allow 8000/tcp comment 'Moatless Vibe App'
    
    # Allow HTTP and HTTPS (for future use)
    sudo ufw allow 80/tcp comment 'HTTP'
    sudo ufw allow 443/tcp comment 'HTTPS'
    
    # Enable firewall
    sudo ufw --force enable
    
    # Show status
    sudo ufw status verbose
}

# Create application directories
setup_application() {
    print_info "Setting up application environment..."
    
    # Add current user to docker group
    sudo usermod -aG docker $USER
    
    # Create application directories
    sudo mkdir -p /opt/moatless-vibe
    sudo mkdir -p /var/lib/moatless-vibe/data
    sudo mkdir -p /var/log/moatless-vibe
    
    # Set permissions for current user
    sudo chown -R $USER:$USER /opt/moatless-vibe
    sudo chown -R $USER:$USER /var/lib/moatless-vibe
    sudo chown -R $USER:$USER /var/log/moatless-vibe
    
    # Create systemd directory for user
    sudo mkdir -p /etc/systemd/system
    
    print_info "Application directories created"
}

# Install additional utilities
install_utilities() {
    print_info "Installing additional utilities..."
    
    # Install useful tools
    sudo apt-get install -y \
        htop \
        tmux \
        jq \
        zip \
        unzip \
        nginx \
        certbot \
        python3-certbot-nginx
    
    # Stop nginx for now (will be configured later if needed)
    sudo systemctl stop nginx
    sudo systemctl disable nginx
}

# Create swap file (useful for smaller VMs)
create_swap() {
    print_info "Creating swap file (4GB)..."
    
    if [ ! -f /swapfile ]; then
        sudo fallocate -l 4G /swapfile
        sudo chmod 600 /swapfile
        sudo mkswap /swapfile
        sudo swapon /swapfile
        echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
        
        # Optimize swappiness
        echo 'vm.swappiness=10' | sudo tee -a /etc/sysctl.conf
        sudo sysctl -p
        
        print_info "Swap file created and activated"
    else
        print_warning "Swap file already exists"
    fi
}

# Main execution
main() {
    echo "========================================="
    echo "VM Initialization for Moatless Vibe"
    echo "========================================="
    echo
    
    # Check if running as root
    if [ "$EUID" -eq 0 ]; then
        print_error "Please run this script as a regular user, not as root"
        exit 1
    fi
    
    # Run installation steps
    update_system
    install_nodejs
    install_bun
    install_java_maven
    install_docker
    configure_firewall
    setup_application
    install_utilities
    create_swap
    
    echo
    print_info "========================================="
    print_info "VM INITIALIZATION COMPLETE!"
    print_info "========================================="
    echo
    echo "Installed components:"
    echo "  - Node.js 22.x"
    echo "  - Bun package manager"
    echo "  - Java 17 (OpenJDK)"
    echo "  - Maven"
    echo "  - Docker & Docker Compose"
    echo "  - Git, build tools, and utilities"
    echo
    echo "Application directories:"
    echo "  - /opt/moatless-vibe (application code)"
    echo "  - /var/lib/moatless-vibe/data (persistent data)"
    echo "  - /var/log/moatless-vibe (logs)"
    echo
    echo "Firewall rules configured:"
    echo "  - SSH (22)"
    echo "  - Application (8000)"
    echo "  - HTTP (80)"
    echo "  - HTTPS (443)"
    echo
    print_warning "IMPORTANT: Log out and back in for docker group membership to take effect"
    echo
    echo "Next steps:"
    echo "1. Log out and back in (for docker permissions)"
    echo "2. Run the deployment script: ./deploy-app.sh"
}

# Run main function
main "$@"