#!/usr/bin/env bash
# Android smoke test, run on a booted emulator: installs and launches the
# APK, and saves what shows whether it works: a screenshot, the log, and the
# window manager's view of the windows and system bars.
#
# Usage: android-smoke.sh <apk> <output dir>
set -euo pipefail

apk=$1
out=$2
package=io.github.haraldreingruber.resume
mkdir -p "$out"

# Three-button navigation, as on many phones (emulators default to gestures).
adb shell cmd overlay enable com.android.internal.systemui.navbar.threebutton || true
adb install -r "$apk"
adb logcat -c
adb shell am start -W -n "$package/android.app.NativeActivity"
sleep 20

adb exec-out screencap -p > "$out/emulator.png"
adb logcat -d > "$out/logcat.txt"
adb shell dumpsys window windows > "$out/dumpsys-window.txt"
adb shell getprop ro.build.version.sdk > "$out/api-level.txt"

echo "--- app log (API level $(cat "$out/api-level.txt"))"
grep -E "resume-3d|AndroidRuntime|FATAL" "$out/logcat.txt" | tail -60 || true
echo "--- the app's window"
grep -A40 "Window{.*$package" "$out/dumpsys-window.txt" \
  | grep -iE "Window\{|requestedVisib|insets|systemUi|mAttrs" | head -30 || true

# Still running? (A crash would have ended it.)
if ! adb shell pidof "$package" > /dev/null; then
  echo "::error::The app isn't running any more."
  exit 1
fi
