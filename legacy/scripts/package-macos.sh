#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build -p sims-app --bin sims-inspector
bundle='target/debug/Sims Archive Inspector.app'
mkdir -p "$bundle/Contents/MacOS"
cp target/debug/sims-inspector "$bundle/Contents/MacOS/sims-inspector"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Sims Archive Inspector</string>
<key>CFBundleDisplayName</key><string>Sims Archive Inspector</string>
<key>CFBundleIdentifier</key><string>local.sims-rs.inspector</string>
<key>CFBundleExecutable</key><string>sims-inspector</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf '%s\n' "Created $bundle"
