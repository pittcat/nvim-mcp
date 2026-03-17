#!/bin/bash
#
# Neovim MCP Server Installation Script
# Supports both source build and existing binary installation methods
# HTTP mode with auto-discovery of Neovim sockets
#

set -e

# Configuration
APP_NAME="nvim-mcp"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${DATA_DIR:-$HOME/.local/share/nvim-mcp}"
LOG_DIR="${DATA_DIR}/logs"

# Default HTTP configuration
HTTP_PORT="${NVIM_MCP_HTTP_PORT:-8080}"
HTTP_HOST="${NVIM_MCP_HTTP_HOST:-127.0.0.1}"
CONNECT_MODE="${NVIM_MCP_CONNECT:-auto}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print functions
info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Usage
usage() {
    cat << 'EOF'
Usage: ./install.sh [OPTIONS]

Options:
    -b, --binary PATH       Use existing binary path, skip source build
    -s, --skip-build        Skip build, assume binary is in ~/.local/bin/
    -i, --install-dir DIR   Install directory (default: ~/.local/bin)
    -p, --port PORT         HTTP server port (default: 8080)
    --host HOST             HTTP server host (default: 127.0.0.1)
    --connect MODE          Connection mode: auto, or socket path (default: auto)
    -u, --uninstall         Uninstall service
    -h, --help              Show this help

Examples:
    # Build from source and install
    ./install.sh

    # Use an existing binary
    ./install.sh --binary ./target/release/nvim-mcp

    # Binary already in ~/.local/bin/, configure service only
    ./install.sh --skip-build

    # Custom HTTP port
    ./install.sh --port 9090

    # Custom install directory
    ./install.sh --install-dir /usr/local/bin

    # Uninstall
    ./install.sh --uninstall

EOF
}

# Detect platform
detect_platform() {
    case "$(uname -s)" in
        Darwin*)
            PLATFORM="macos"
            SERVICE_TYPE="launchd"
            ;;
        Linux*)
            PLATFORM="linux"
            if command -v systemctl &> /dev/null; then
                SERVICE_TYPE="systemd"
            else
                error "Linux systems require systemd"
                exit 1
            fi
            ;;
        *)
            error "Unsupported operating system: $(uname -s)"
            exit 1
            ;;
    esac
    info "Detected platform: $PLATFORM ($SERVICE_TYPE)"
}

# Check if directory exists and create
ensure_dir() {
    if [ ! -d "$1" ]; then
        mkdir -p "$1"
        success "Created directory: $1"
    fi
}

# Check dependencies
check_dependencies() {
    info "Checking dependencies..."

    # Check Neovim
    if ! command -v nvim &> /dev/null; then
        warn "Neovim not found in PATH. Auto-connect may not work properly."
    else
        success "Neovim: $(nvim --version | head -1)"
    fi

    # Check Rust if building from source
    if [ "$SKIP_BUILD" != "true" ] && [ -z "$BINARY_PATH" ]; then
        if ! command -v cargo &> /dev/null; then
            error "Rust/Cargo not found, please install: https://rustup.rs/"
            exit 1
        fi
        success "Rust: $(cargo --version)"
    fi
}

# Check if a newer version exists in Cargo.toml
check_version() {
    local installed_version=""
    local source_version=""

    # Get installed version
    if [ -x "$INSTALL_DIR/$APP_NAME" ]; then
        installed_version=$("$INSTALL_DIR/$APP_NAME" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || echo "unknown")
    fi

    # Get source version from Cargo.toml
    if [ -f "Cargo.toml" ]; then
        source_version=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    fi

    if [ -n "$installed_version" ] && [ "$installed_version" != "unknown" ]; then
        if [ -n "$source_version" ]; then
            if [ "$installed_version" = "$source_version" ]; then
                info "Version $installed_version already installed"
                return 0
            else
                warn "Version mismatch: installed=$installed_version, source=$source_version"
                read -p "Overwrite with new version? [Y/n] " -n 1 -r
                echo
                if [[ $REPLY =~ ^[Nn]$ ]]; then
                    info "Installation cancelled"
                    exit 0
                fi
            fi
        else
            warn "Already installed: $APP_NAME $installed_version"
            read -p "Reinstall/overwrite? [Y/n] " -n 1 -r
            echo
            if [[ $REPLY =~ ^[Nn]$ ]]; then
                info "Installation cancelled"
                exit 0
            fi
        fi
    fi
}

# Build from source
build_from_source() {
    info "Building from source..."

    # Check if in project directory
    if [ ! -f "Cargo.toml" ]; then
        error "Cargo.toml not found, please run this script from the project root"
        exit 1
    fi

    # Check version before build
    check_version

    # Build release version
    cargo build --release

    if [ ! -f "target/release/$APP_NAME" ]; then
        error "Build failed: target/release/$APP_NAME not found"
        exit 1
    fi

    BINARY_PATH="target/release/$APP_NAME"
    success "Build completed: $BINARY_PATH"
}

# Install binary
install_binary() {
    info "Installing binary..."

    local src="$1"
    local dst="$INSTALL_DIR/$APP_NAME"

    # Skip if source is already the destination
    if [ "$src" = "$dst" ]; then
        info "Binary already in target location, skipping copy"
        return
    fi

    # Backup existing binary if exists
    if [ -f "$dst" ]; then
        info "Backing up existing binary..."
        mv "$dst" "$dst.backup.$(date +%Y%m%d%H%M%S)"
    fi

    # Copy and set permissions
    cp "$src" "$dst"
    chmod +x "$dst"
    success "Installed: $dst"
}

# Check and release port
ensure_port_available() {
    info "Checking if port $HTTP_PORT is available..."

    # Check port usage (get all PIDs)
    local pids
    pids=$(lsof -ti :"$HTTP_PORT" 2>/dev/null | tr '\n' ' ' || true)

    if [ -n "$pids" ]; then
        warn "Port $HTTP_PORT is in use by process: $pids"

        # Check if it's our service
        case "$SERVICE_TYPE" in
            systemd)
                if systemctl --user is-active --quiet "$APP_NAME.service" 2>/dev/null; then
                    info "Stopping existing systemd service..."
                    systemctl --user stop "$APP_NAME.service" 2>/dev/null || true
                    systemctl --user disable "$APP_NAME.service" 2>/dev/null || true
                    sleep 2
                fi
                ;;
            launchd)
                if launchctl list | grep -q "com.github.linw1995.$APP_NAME"; then
                    info "Stopping existing launchd service..."
                    launchctl stop "com.github.linw1995.$APP_NAME" 2>/dev/null || true
                    launchctl unload "$HOME/Library/LaunchAgents/com.github.linw1995.$APP_NAME.plist" 2>/dev/null || true
                    sleep 2
                fi
                ;;
        esac

        # Check again
        pids=$(lsof -ti :"$HTTP_PORT" 2>/dev/null | tr '\n' ' ' || true)
        if [ -n "$pids" ]; then
            info "Force killing process: $pids..."
            for pid in $pids; do
                kill -9 "$pid" 2>/dev/null || true
            done
            sleep 2
        fi

        # Final check
        if lsof -ti :"$HTTP_PORT" >/dev/null 2>&1; then
            error "Cannot release port $HTTP_PORT, please check manually: lsof -i :$HTTP_PORT"
            exit 1
        fi

        success "Port $HTTP_PORT released"
    else
        success "Port $HTTP_PORT is free"
    fi
}

# Create systemd user service (Linux)
setup_systemd_service() {
    info "Configuring systemd user service..."

    # Ensure port is available first
    ensure_port_available

    local service_dir="$HOME/.config/systemd/user"
    local service_file="$service_dir/$APP_NAME.service"

    ensure_dir "$service_dir"

    cat > "$service_file" << EOF
[Unit]
Description=Neovim MCP Server
Documentation=https://github.com/linw1995/nvim-mcp
After=network.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/$APP_NAME --http-port $HTTP_PORT --http-host $HTTP_HOST --connect $CONNECT_MODE
Restart=on-failure
RestartSec=5
Environment="RUST_LOG=info"
Environment="NVIM_MCP_HTTP_PORT=$HTTP_PORT"
Environment="NVIM_MCP_HTTP_HOST=$HTTP_HOST"
Environment="NVIM_MCP_CONNECT=$CONNECT_MODE"

[Install]
WantedBy=default.target
EOF

    success "Created service file: $service_file"

    # Reload systemd
    systemctl --user daemon-reload

    # Enable and start service
    systemctl --user enable "$APP_NAME.service"
    systemctl --user restart "$APP_NAME.service"

    # Wait for service to start
    sleep 2

    if systemctl --user is-active --quiet "$APP_NAME.service"; then
        success "Service started and enabled for auto-start"
    else
        error "Service failed to start, check logs: journalctl --user -u $APP_NAME"
        exit 1
    fi
}

# Create launchd service (macOS)
setup_launchd_service() {
    info "Configuring launchd service..."

    # Ensure port is available first
    ensure_port_available

    local plist_dir="$HOME/Library/LaunchAgents"
    local plist_file="$plist_dir/com.github.linw1995.$APP_NAME.plist"

    ensure_dir "$plist_dir"

    cat > "$plist_file" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.github.linw1995.$APP_NAME</string>

    <key>ProgramArguments</key>
    <array>
        <string>$INSTALL_DIR/$APP_NAME</string>
        <string>--http-port</string>
        <string>$HTTP_PORT</string>
        <string>--http-host</string>
        <string>$HTTP_HOST</string>
        <string>--connect</string>
        <string>$CONNECT_MODE</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
        <key>NVIM_MCP_HTTP_PORT</key>
        <string>$HTTP_PORT</string>
        <key>NVIM_MCP_HTTP_HOST</key>
        <string>$HTTP_HOST</string>
        <key>NVIM_MCP_CONNECT</key>
        <string>$CONNECT_MODE</string>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
    </dict>

    <key>KeepAlive</key>
    <true/>

    <key>RunAtLoad</key>
    <true/>

    <key>ThrottleInterval</key>
    <integer>5</integer>

    <key>StandardOutPath</key>
    <string>$LOG_DIR/server.log</string>

    <key>StandardErrorPath</key>
    <string>$LOG_DIR/server.error.log</string>
</dict>
</plist>
EOF

    success "Created plist file: $plist_file"

    # Load and start service
    launchctl unload "$plist_file" 2>/dev/null || true
    launchctl load "$plist_file"
    launchctl start "com.github.linw1995.$APP_NAME" 2>/dev/null || true

    # Wait for service to start
    sleep 2

    # Check service status
    if launchctl list | grep -q "com.github.linw1995.$APP_NAME"; then
        success "Service started and enabled for auto-start"
    else
        warn "Service may not have started, please check manually: launchctl list | grep $APP_NAME"
    fi
}

# Setup service
setup_service() {
    info "Configuring auto-start service..."

    case "$SERVICE_TYPE" in
        systemd)
            setup_systemd_service
            ;;
        launchd)
            setup_launchd_service
            ;;
    esac
}

# Uninstall service
uninstall() {
    info "Uninstalling $APP_NAME..."

    detect_platform

    case "$SERVICE_TYPE" in
        systemd)
            systemctl --user stop "$APP_NAME.service" 2>/dev/null || true
            systemctl --user disable "$APP_NAME.service" 2>/dev/null || true
            rm -f "$HOME/.config/systemd/user/$APP_NAME.service"
            systemctl --user daemon-reload
            ;;
        launchd)
            launchctl stop "com.github.linw1995.$APP_NAME" 2>/dev/null || true
            launchctl unload "$HOME/Library/LaunchAgents/com.github.linw1995.$APP_NAME.plist" 2>/dev/null || true
            rm -f "$HOME/Library/LaunchAgents/com.github.linw1995.$APP_NAME.plist"
            ;;
    esac

    # Remove binary
    if [ -f "$INSTALL_DIR/$APP_NAME" ]; then
        rm -f "$INSTALL_DIR/$APP_NAME"
        success "Removed binary: $INSTALL_DIR/$APP_NAME"
    fi

    # Ask whether to delete data
    read -p "Delete data and log directories? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        rm -rf "$DATA_DIR"
        success "Deleted data directory: $DATA_DIR"
    fi

    success "Uninstall completed"
}

# Verify installation
verify_installation() {
    info "Verifying installation..."

    # Check binary
    if [ ! -x "$INSTALL_DIR/$APP_NAME" ]; then
        error "Binary file does not exist or is not executable"
        exit 1
    fi

    # Get installed version
    local version
    version=$("$INSTALL_DIR/$APP_NAME" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || echo "unknown")
    info "Installed version: $version"

    # Wait for service to be ready
    sleep 2

    # Test connection
    local retry=0
    local max_retry=5
    local service_ready=false

    while [ $retry -lt $max_retry ]; do
        if curl -s "http://${HTTP_HOST}:${HTTP_PORT}/mcp" > /dev/null 2>&1 || \
           curl -s "http://${HTTP_HOST}:${HTTP_PORT}/health" > /dev/null 2>&1 || \
           curl -s "http://${HTTP_HOST}:${HTTP_PORT}" > /dev/null 2>&1; then
            service_ready=true
            break
        fi
        retry=$((retry + 1))
        info "Waiting for service to start... ($retry/$max_retry)"
        sleep 1
    done

    if [ "$service_ready" = true ]; then
        success "Service responding normally at http://${HTTP_HOST}:${HTTP_PORT}"
        # Try to get tools list
        local tools_count
        tools_count=$(curl -s "http://${HTTP_HOST}:${HTTP_PORT}/mcp/tools" 2>/dev/null | grep -o '"name"' | wc -l | tr -d ' ' || echo "0")
        if [ -n "$tools_count" ] && [ "$tools_count" -gt 0 ]; then
            info "Available tools count: $tools_count"
        fi
    else
        warn "Service may still be starting, please check manually later"
        warn "Check logs: tail -f $LOG_DIR/server.log"
    fi

    success "Installation verification completed!"
}

# Print installation info
print_info() {
    echo
    echo "========================================"
    echo "  Neovim MCP Server Installation Complete!"
    echo "========================================"
    echo
    echo "  Install path: $INSTALL_DIR/$APP_NAME"
    echo "  Log file: $LOG_DIR/server.log"
    echo "  Service address: http://$HTTP_HOST:$HTTP_PORT"
    echo "  Connect mode: $CONNECT_MODE"
    echo
    echo "  Common commands:"
    case "$SERVICE_TYPE" in
        systemd)
            echo "    Check status: systemctl --user status $APP_NAME"
            echo "    View logs: journalctl --user -u $APP_NAME -f"
            echo "    Restart service: systemctl --user restart $APP_NAME"
            echo "    Stop service: systemctl --user stop $APP_NAME"
            ;;
        launchd)
            echo "    Check status: launchctl list | grep $APP_NAME"
            echo "    View logs: tail -f $LOG_DIR/server.log"
            echo "    Restart service: launchctl stop com.github.linw1995.$APP_NAME; launchctl start com.github.linw1995.$APP_NAME"
            echo "    Stop service: launchctl stop com.github.linw1995.$APP_NAME"
            ;;
    esac
    echo
    echo "  MCP Client configuration:"
    echo "    Add to your MCP client config:"
    echo "    {"
    echo "      \"mcpServers\": {"
    echo "        \"nvim\": {"
    echo "          \"url\": \"http://$HTTP_HOST:$HTTP_PORT/mcp\""
    echo "        }"
    echo "      }"
    echo "    }"
    echo
    echo "========================================"
}

# Main function
main() {
    # Parse arguments
    SKIP_BUILD="false"
    BINARY_PATH=""
    UNINSTALL_MODE="false"

    while [[ $# -gt 0 ]]; do
        case $1 in
            -b|--binary)
                BINARY_PATH="$2"
                shift 2
                ;;
            -s|--skip-build)
                SKIP_BUILD="true"
                shift
                ;;
            -i|--install-dir)
                INSTALL_DIR="$2"
                shift 2
                ;;
            -p|--port)
                HTTP_PORT="$2"
                shift 2
                ;;
            --host)
                HTTP_HOST="$2"
                shift 2
                ;;
            --connect)
                CONNECT_MODE="$2"
                shift 2
                ;;
            -u|--uninstall)
                UNINSTALL_MODE="true"
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done

    # Uninstall mode
    if [ "$UNINSTALL_MODE" = "true" ]; then
        uninstall
        exit 0
    fi

    echo "========================================"
    echo "  Neovim MCP Server Installation Script"
    echo "========================================"
    echo

    # Detect platform
    detect_platform

    # Check dependencies
    check_dependencies

    # Create required directories
    ensure_dir "$INSTALL_DIR"
    ensure_dir "$DATA_DIR"
    ensure_dir "$LOG_DIR"

    # Get binary file
    if [ -n "$BINARY_PATH" ]; then
        # Use specified binary file
        if [ ! -f "$BINARY_PATH" ]; then
            error "Specified binary file does not exist: $BINARY_PATH"
            exit 1
        fi
        info "Using specified binary: $BINARY_PATH"
        install_binary "$BINARY_PATH"
    elif [ "$SKIP_BUILD" = "true" ]; then
        # Skip build, check if already installed
        if [ ! -f "$INSTALL_DIR/$APP_NAME" ]; then
            error "No installed binary found: $INSTALL_DIR/$APP_NAME"
            info "Please use --binary to specify binary path, or remove --skip-build to build from source"
            exit 1
        fi
        info "Using already installed binary"
    else
        # Build from source
        build_from_source
        install_binary "$BINARY_PATH"
    fi

    # Configure service
    setup_service

    # Verify installation
    verify_installation

    # Print info
    print_info
}

main "$@"
