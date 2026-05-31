#!/bin/bash
set -euo pipefail

# Build Flovenet .deb package
# Usage: ./scripts/build-deb.sh [version]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
VERSION="${1:-$(cd "$PROJECT_DIR" && grep '^version =' daemon/Cargo.toml | head -1 | cut -d'"' -f2)}"
DEB_DIR="$PROJECT_DIR/deb-pkg"
BUILD_DIR="$PROJECT_DIR/target/deb"

echo "==> Building flovenet v$VERSION .deb package"

# Step 1: Build the release binary
echo "==> Compiling flovenet daemon..."
cd "$PROJECT_DIR"
cargo build --release --bin daemon

# Step 2: Prepare deb directory
echo "==> Preparing package structure..."
rm -rf "$BUILD_DIR"
PKG_DIR="$BUILD_DIR/flovenet_${VERSION}_amd64"
mkdir -p "$PKG_DIR"

# Step 3: Copy control files
cp -r "$DEB_DIR/DEBIAN" "$PKG_DIR/"
cp -r "$DEB_DIR/lib" "$PKG_DIR/"
mkdir -p "$PKG_DIR/usr/bin"
mkdir -p "$PKG_DIR/usr/share/doc/flovenet"
mkdir -p "$PKG_DIR/usr/share/man/man1"

# Step 4: Copy binary
cp "$PROJECT_DIR/target/release/daemon" "$PKG_DIR/usr/bin/flovenet"
strip "$PKG_DIR/usr/bin/flovenet" 2>/dev/null || true

# Step 5: Copy docs
cp "$PROJECT_DIR/README.md" "$PKG_DIR/usr/share/doc/flovenet/"

# Step 6: Generate man page
cat > "$PKG_DIR/usr/share/man/man1/flovenet.1" << 'MANEOF'
.TH FLOVENET 1 "2026" "Flovenet" "User Commands"
.SH NAME
flovenet \- decentralized compute sharing network
.SH SYNOPSIS
.B flovenet
[\fIdaemon\fR|\fIapi-gateway\fR|\fIshare\fR|\fIrun\fR|\fIstatus\fR]
.SH DESCRIPTION
Flovenet is a P2P network for sharing computing resources.
.SH COMMANDS
.TP
.B daemon
Start a P2P node with specified roles.
.TP
.B api-gateway
Start the GraphQL API gateway.
.TP
.B share
Display resource information for a role.
.TP
.B run
Execute a WASM job locally.
.TP
.B status
Display node resource information.
MANEOF
gzip -9 "$PKG_DIR/usr/share/man/man1/flovenet.1"

# Step 7: Update version in control file
sed -i "s/Version: .*/Version: $VERSION/" "$PKG_DIR/DEBIAN/control"

# Step 8: Build .deb
echo "==> Building .deb package..."
fakeroot dpkg-deb --build "$PKG_DIR" "$PROJECT_DIR/target/flovenet_${VERSION}_amd64.deb"

echo "==> Done!"
echo "    Package: $PROJECT_DIR/target/flovenet_${VERSION}_amd64.deb"
echo "    Install: sudo dpkg -i target/flovenet_${VERSION}_amd64.deb"
