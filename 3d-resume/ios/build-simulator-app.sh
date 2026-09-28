#!/usr/bin/env bash
# Builds the iOS Simulator app (Apple silicon Macs) as a zipped
# "3D Resume.app": the program for aarch64-apple-ios-sim plus Info.plist,
# ad-hoc signed. No Apple developer account needed; it runs only in the
# simulator:
#   unzip resume-3d-ios-simulator.zip
#   xcrun simctl install booted "3D Resume.app"
#   xcrun simctl launch booted io.github.haraldreingruber.resume
#
# Needs: macOS with Xcode, the Rust target aarch64-apple-ios-sim.
# Usage: build-simulator-app.sh <output.zip>
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
out="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$here/../.."

cargo build -p resume-3d --release --target aarch64-apple-ios-sim

version=$(sed -n 's/^version = "\(.*\)"/\1/p' 3d-resume/Cargo.toml | head -1)
app="$work/3D Resume.app"
mkdir -p "$app"
cp target/aarch64-apple-ios-sim/release/resume-3d "$app/"
sed -e "s/@VERSION@/$version/" -e "s/@BUILD@/${GITHUB_RUN_NUMBER:-1}/" "$here/Info.plist" > "$app/Info.plist"
plutil -lint "$app/Info.plist"
codesign --force --sign - "$app"
rm -f "$out"
(cd "$work" && ditto -c -k --keepParent "3D Resume.app" "$out")
echo "Built $out ($(wc -c < "$out") bytes)"
