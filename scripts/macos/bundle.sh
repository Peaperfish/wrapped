#!/bin/sh
# Build a minimal, ad-hoc signed Wrapped.app around the release collector,
# with the dashboard nested inside it:
#
#   Wrapped.app                                   collector (menu bar, LSUIElement)
#     Contents/Helpers/Wrapped Dashboard.app      dashboard (normal Dock app,
#                                                 runs only while its window is open)
#
# The outer bundle is needed for launch at login (SMAppService.mainApp
# registers the app the running executable is inside). Both carry the same
# version: CFBundleVersion is the build id (short git hash, "-dirty" if the
# tree has uncommitted changes). Output: target/release/Wrapped.app
set -eu
cd "$(dirname "$0")/../.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/collector/Cargo.toml | head -1)
# Build id: short git hash, "-dirty" if the tree has uncommitted changes.
BUILD=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then BUILD="$BUILD-dirty"; fi
export WRAPPED_BUILD_ID="$BUILD"

# Dashboard frontend (dashboard/ui/dist, embedded into the dashboard binary).
# Reinstall node modules only when the lockfile changed.
(
    cd dashboard/ui
    if [ ! -f node_modules/.package-lock.json ] || [ package-lock.json -nt node_modules/.package-lock.json ]; then
        npm ci --no-audit --no-fund --loglevel=error
    fi
    npm run --silent build >/dev/null
)

# Two cargo runs, not one: features are unified across the packages of one
# build, and the collector must not pick up any the dashboard's deps enable.
cargo build -q --release -p wrapped-collector
cargo build -q --release -p wrapped-dashboard --features custom-protocol

APP=target/release/Wrapped.app
DASH="$APP/Contents/Helpers/Wrapped Dashboard.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$DASH/Contents/MacOS" "$DASH/Contents/Resources"
cp target/release/wrapped-collector "$APP/Contents/MacOS/wrapped-collector"
cp target/release/wrapped-dashboard "$DASH/Contents/MacOS/wrapped-dashboard"
cp dashboard/icons/icon.icns "$DASH/Contents/Resources/icon.icns"

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

cat > "$DASH/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>         <string>io.github.peaperfish.wrapped.dashboard</string>
    <key>CFBundleName</key>               <string>Wrapped Dashboard</string>
    <key>CFBundleDisplayName</key>        <string>Wrapped</string>
    <key>CFBundleExecutable</key>         <string>wrapped-dashboard</string>
    <key>CFBundleIconFile</key>           <string>icon</string>
    <key>CFBundlePackageType</key>        <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>$VERSION</string>
    <key>CFBundleVersion</key>            <string>$BUILD</string>
    <key>LSMinimumSystemVersion</key>     <string>13.0</string>
    <key>NSHighResolutionCapable</key>    <true/>
</dict>
</plist>
PLIST

# Ad-hoc signatures (no developer account needed): the nested app first,
# then the outer bundle, whose signature seals the nested one.
codesign --force --sign - "$DASH"
codesign --force --sign - "$APP"
echo "built $APP ($BUILD, dashboard nested)"
