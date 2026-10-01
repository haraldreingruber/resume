#!/usr/bin/env bash
# Android smoke test, run on a booted emulator: installs and launches the
# APK, and saves what shows whether it works: a screenshot, the log, and the
# window manager's view of the windows and system bars. Fails if the app
# stopped or froze, didn't hide the system bars, or a link opened nothing.
#
# Usage: android-smoke.sh <apk> <output dir>
set -euo pipefail

apk=$1
out=$2
package=io.github.haraldreingruber.resume
activity=$package/android.app.NativeActivity
mkdir -p "$out"

# A freshly booted emulator stays busy for a while, more so on CI without a
# GPU. The launcher, which draws the navigation buttons, then lags: it
# showed them long after the app had hidden them (a tap hit its home
# button), or froze ("isn't responding", a dialog that swallowed the tap).
# So: let the system settle, and don't let other apps' freezes show dialogs
# (Android restarts them instead; a freeze of this app still fails the test,
# see below). The one-time "Viewing full screen" notice is confirmed
# beforehand for the same reason.
adb shell settings put global hide_error_dialogs 1
adb shell settings put secure immersive_mode_confirmations confirmed
# Three-button navigation, as on many phones (emulators default to gestures).
adb shell cmd overlay enable com.android.internal.systemui.navbar.threebutton || true
adb shell am wait-for-broadcast-idle > /dev/null || true
sleep 15

adb install -r "$apk"
adb logcat -c
adb shell am start -W -n "$activity"
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

# Whether the app's activity is in front (not, e.g., the launcher). (Not
# `| grep -q`: with pipefail, an early match can fail the pipe.)
in_front() {
  local resumed
  resumed=$(adb shell dumpsys activity activities | grep -E "topResumedActivity|mResumedActivity" || true)
  [[ $resumed == *"$package"* ]]
}

# A link: tap "Text version" (bottom right, where it is in this emulator's
# 320x640 portrait layout); Android must start an activity for the URL. A
# tap can still land on something of the system's (see above), so up to
# three tries, with the app brought back to the front first.
read -r width height < <(adb shell wm size | tr -d '\r' | awk -F'[ x]' '/Physical/ { print $3, $4 }')
opened=0
for attempt in 1 2 3; do
  if ! in_front; then
    echo "The app isn't in front; bringing it back."
    adb shell am start -W -n "$activity"
    sleep 10
  fi
  adb logcat -c
  adb shell input tap $((width * 61 / 100)) $((height * 944 / 1000))
  sleep 5
  adb logcat -d > "$out/logcat-link-$attempt.txt"
  adb exec-out screencap -p > "$out/emulator-link-$attempt.png"
  echo "--- after tapping Text version (try $attempt)"
  grep -E "resume-3d|act=android.intent.action.VIEW|LAUNCHER_TASKBAR" "$out/logcat-link-$attempt.txt" \
    | tail -10 || true
  if grep -q "act=android.intent.action.VIEW" "$out/logcat-link-$attempt.txt"; then
    opened=1
    break
  fi
done
if [ "$opened" -eq 0 ]; then
  echo "::error::Tapping Text version started no activity for its URL (3 tries)."
  failed=1
fi

# A freeze of this app (input not handled for 5 s) would be hidden like
# the others' (see above), so look for it in the log.
if grep -qE "ANR in $package" "$out"/logcat*.txt; then
  echo "::error::The app froze (Android reported it as not responding)."
  failed=1
fi
exit $failed
