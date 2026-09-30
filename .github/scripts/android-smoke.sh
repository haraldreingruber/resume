#!/usr/bin/env bash
# Android smoke test, run on a booted emulator: installs and launches the
# APK, and saves what shows whether it works: a screenshot, the log, and the
# window manager's view of the windows and system bars. Fails if the app
# stopped, didn't hide the system bars, or a link opened nothing.
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
adb shell dumpsys window > "$out/dumpsys-window.txt"
adb shell getprop ro.build.version.sdk > "$out/api-level.txt"

echo "--- app log (API level $(cat "$out/api-level.txt"))"
grep -E "resume-3d|AndroidRuntime|FATAL" "$out/logcat.txt" | tail -60 || true
echo "--- system bars (insets sources)"
grep -iE "InsetsSource.*type=(navigationBars|statusBars)" "$out/dumpsys-window.txt" \
  | grep -oE "type=[a-zA-Z]+|visible=[a-z]+" | paste -sd' ' | head -c 2000 || true
echo

failed=0
# Still running? (A crash would have ended it.)
if ! adb shell pidof "$package" > /dev/null; then
  echo "::error::The app isn't running any more."
  exit 1
fi
# Immersive mode: the navigation bar would cover the app's buttons.
if ! grep -q "system bars hidden" "$out/logcat.txt"; then
  echo "::error::The app didn't hide the system bars (see the app log above)."
  failed=1
fi

# A link: tap "Text version" (bottom right, where it is in this emulator's
# 320x640 portrait layout); Android must start an activity for the URL.
read -r width height < <(adb shell wm size | tr -d '\r' | awk -F'[ x]' '/Physical/ { print $3, $4 }')
adb logcat -c
adb shell input tap $((width * 61 / 100)) $((height * 944 / 1000))
sleep 5
adb logcat -d > "$out/logcat-link.txt"
adb exec-out screencap -p > "$out/emulator-link.png"
echo "--- after tapping Text version"
grep -E "resume-3d|act=android.intent.action.VIEW" "$out/logcat-link.txt" | tail -10 || true
if ! grep -q "act=android.intent.action.VIEW" "$out/logcat-link.txt"; then
  echo "::error::Tapping Text version started no activity for its URL."
  failed=1
fi
exit $failed
