#!/bin/sh
# Build a minimal, ad-hoc signed Wrapped.app around the release collector.
# The bundle is needed for launch at login (SMAppService.mainApp registers
# the app the running executable is inside) and hides the Dock icon via
# LSUIElement. Output: target/release/Wrapped.app
set -eu
cd "$(dirname "$0")/../.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/collector/Cargo.toml | head -1)
# Build id: short git hash, "-dirty" if the tree has uncommitted changes.
BUILD=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then BUILD="$BUILD-dirty"; fi
export WRAPPED_BUILD_ID="$BUILD"

cargo build -q --release -p wrapped-collector

APP=target/release/Wrapped.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/wrapped-collector "$APP/Contents/MacOS/wrapped-collector"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>         <string>io.github.peaperfish.wrapped</string>
    <key>CFBundleName</key>               <string>Wrapped</string>
    <key>CFBundleExecutable</key>         <string>wrapped-collector</string>
    <key>CFBundlePackageType</key>        <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>$VERSION</string>
    <key>CFBundleVersion</key>            <string>$BUILD</string>
    <key>LSMinimumSystemVersion</key>     <string>13.0</string>
    <key>LSUIElement</key>                <true/>
</dict>
</plist>
PLIST
# Ad-hoc signature covering Info.plist (no developer account needed).
codesign --force --sign - "$APP"
echo "built $APP"
