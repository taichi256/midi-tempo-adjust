#!/bin/bash
# Finderから使える .app バンドルを作る
set -e
cd "$(dirname "$0")"
APP="MIDI頭出しツール.app"
cargo build --release
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/midi_head_align "$APP/Contents/MacOS/midi_head_align"
mkdir -p "$APP/Contents/Resources/lang"
cp lang/*.json "$APP/Contents/Resources/lang/" 2>/dev/null || true
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>MIDI Head Align</string>
  <key>CFBundleDisplayName</key><string>MIDI 頭出しツール</string>
  <key>CFBundleIdentifier</key><string>local.midi-head-align</string>
  <key>CFBundleExecutable</key><string>midi_head_align</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>10.15</string>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP" 2>/dev/null || true
echo "作成しました: $APP"
