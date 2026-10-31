#!/usr/bin/env bash
set -e

INSTALL_DIR="$HOME/.local/share/strawberry-social"
BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="$HOME/.local/share/applications"

echo "Installing Strawberry Social..."

mkdir -p "$INSTALL_DIR/characters"
mkdir -p "$INSTALL_DIR/sessions"
mkdir -p "$BIN_DIR"
mkdir -p "$DESKTOP_DIR"

# Locate and copy binary
if [ -f "target/x86_64-unknown-linux-gnu/release/strawberry-social" ]; then
    cp target/x86_64-unknown-linux-gnu/release/strawberry-social "$INSTALL_DIR/"
elif [ -f "strawberry-social" ]; then
    cp strawberry-social "$INSTALL_DIR/"
else
    echo "Error: strawberry-social binary not found. Build it first!"
    exit 1
fi

# Copy config if not already present
if [ -f "config.yaml" ] && [ ! -f "$INSTALL_DIR/config.yaml" ]; then
    cp config.yaml "$INSTALL_DIR/"
fi

chmod +x "$INSTALL_DIR/strawberry-social"

# Create a wrapper script in ~/.local/bin to guarantee correct working directory
cat << EOF > "$BIN_DIR/strawberry-social"
#!/usr/bin/env bash
cd "$INSTALL_DIR"
exec ./strawberry-social "\$@"
EOF
chmod +x "$BIN_DIR/strawberry-social"

# Create a desktop shortcut entry
cat << EOF > "$DESKTOP_DIR/strawberry-social.desktop"
[Desktop Entry]
Type=Application
Name=Strawberry Social
Comment=Lightweight AI Chat
Exec=$BIN_DIR/strawberry-social
Icon=utilities-terminal
Categories=Utility;
Terminal=true
EOF

echo "Installation complete."
echo "Installed to: $INSTALL_DIR"
echo "You can run it from your application menu or via the 'strawberry-social' command."
