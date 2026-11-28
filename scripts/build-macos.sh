#!/bin/sh
# Builds Nuvio.app, self-contained with libmpv and its libraries, and wraps it in
# dist/Nuvio-Rust-<version>-macos-arm64.dmg.
#
# Needs an Apple Silicon Mac, the Rust toolchain, the Xcode command line tools and `brew install mpv`. The app is signed
# ad hoc, without an Apple developer account: macOS asks the person to allow it the first time (see "First start" below).
#
#   scripts/build-macos.sh [--skip-build]
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
PATH="$HOME/.cargo/bin:$PATH"

[ "$(uname -m)" = arm64 ] || { echo "this script builds for Apple Silicon" >&2; exit 1; }
libmpv=${NUVIO_LIBMPV:-$(brew --prefix)/lib/libmpv.2.dylib}
[ -f "$libmpv" ] || { echo "libmpv not found at $libmpv: brew install mpv" >&2; exit 1; }

version=$(sed -n 's/^version *= *"\([^"]*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }

if [ "${1:-}" != --skip-build ]; then
    # Panic messages name source files. Keep the builder's home folder, and so their user name, out of what is shipped.
    export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/build"
    export CFLAGS="${CFLAGS:-} -ffile-prefix-map=$HOME=/build"
    cargo build -p nuvio --release --locked
fi
# cargo may write to a target folder outside the checkout, so ask it.
target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
binary="$target/release/nuvio"
[ -x "$binary" ] || { echo "release build not found: $binary" >&2; exit 1; }
if grep -qaF "$HOME/" "$binary"; then
    echo "$binary names files under $HOME: build it with this script, without --skip-build" >&2
    exit 1
fi

stage="$root/dist/stage"
app="$stage/Nuvio.app"
contents="$app/Contents"
rm -rf "$stage"
mkdir -p "$contents/MacOS" "$contents/Resources"
cp "$binary" "$contents/MacOS/nuvio"
cp LICENSE "$contents/Resources/LICENSE"

newest=$(python3 scripts/bundle-dylibs.py "$libmpv" "$contents/Frameworks")
# Homebrew's builds can name a newer point release than the Mac they were installed and run on, so the app never asks for
# more than this one has. That makes it a build for this Mac's macOS and later.
minimum=$(printf '%s\n%s\n' "$newest" "$(sw_vers -productVersion)" | sort -t. -k1,1n -k2,2n -k3,3n | head -1)

# The versions of what was bundled, for the licences' sake: they are Homebrew's builds.
{
    echo "Libraries in Frameworks, as built by Homebrew (https://github.com/Homebrew/homebrew-core)."
    echo "Their sources and licences are named by each formula."
    echo
    brew list --versions mpv $(brew deps mpv)
} > "$contents/Resources/THIRD-PARTY.txt"

icons="$stage/AppIcon.iconset"
mkdir -p "$icons"
for size in 16 32 128 256; do
    sips -z "$size" "$size" crates/app/resources/icon.png --out "$icons/icon_${size}x${size}.png" >/dev/null
done
sips -z 32 32 crates/app/resources/icon.png --out "$icons/icon_16x16@2x.png" >/dev/null
sips -z 64 64 crates/app/resources/icon.png --out "$icons/icon_32x32@2x.png" >/dev/null
cp "$icons/icon_256x256.png" "$icons/icon_128x128@2x.png"
iconutil -c icns "$icons" -o "$contents/Resources/AppIcon.icns"
rm -rf "$icons"

cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>nuvio</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>io.github.shinigamikira.nuvio-rust</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>Nuvio</string>
    <key>CFBundleDisplayName</key><string>Nuvio</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.entertainment</string>
    <key>LSMinimumSystemVersion</key><string>$minimum</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>GPL-3.0-only</string>
    <key>NSLocalNetworkUsageDescription</key><string>Torrent streams can come from other computers on your network.</string>
</dict>
</plist>
PLIST
plutil -lint "$contents/Info.plist" >/dev/null

# Inside out: editing a library's references broke its signature, and Apple Silicon runs nothing unsigned.
for library in "$contents"/Frameworks/*.dylib; do
    codesign --force --sign - "$library"
done
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

cat > "$stage/First start.txt" <<'TEXT'
Drag Nuvio to Applications.

This build is not signed with an Apple developer certificate, so macOS stops it the first time:

  1. Open Nuvio once and close the warning.
  2. Open System Settings, Privacy & Security, scroll down and choose "Open Anyway".

Or, in Terminal:  xattr -dr com.apple.quarantine /Applications/Nuvio.app
TEXT
ln -s /Applications "$stage/Applications"

dmg="$root/dist/Nuvio-Rust-$version-macos-arm64.dmg"
rm -f "$dmg"
hdiutil create -volname Nuvio -srcfolder "$stage" -format UDZO -quiet "$dmg"
rm -rf "$root/dist/Nuvio.app"
mv "$app" "$root/dist/Nuvio.app"
rm -rf "$stage"

echo "app:  $root/dist/Nuvio.app  (macOS $minimum or newer)"
echo "dmg:  $dmg"
shasum -a 256 "$dmg"
