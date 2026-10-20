#!/usr/bin/env bash
# A smoke test on a real Android device (ROADMAP S3-9): install the debug APK,
# start it, and check what the Android spike found broken stays fixed. It reads
# no screen coordinates, so it does not depend on the phone's size.
#
#   ./android/build.sh && ./tools/android-smoke.sh
#
# Needs ANDROID_HOME and one device visible to adb.
set -uo pipefail
cd "$(dirname "$0")/.."

ADB="${ANDROID_HOME:?set ANDROID_HOME}/platform-tools/adb"
APK=android/app/build/outputs/apk/debug/app-debug.apk
PKG=dev.nimadir.shell
ACTIVITY=$PKG/android.app.NativeActivity
FAIL=0

say()  { printf '  %-6s %s\n' "$1" "$2"; }
fail() { say FAIL "$1"; FAIL=1; }

[[ -f "$APK" ]] || { echo "no $APK — run ./android/build.sh first"; exit 1; }
"$ADB" get-state >/dev/null 2>&1 || { echo "no device visible to adb"; exit 1; }
echo "==> $("$ADB" shell getprop ro.product.model | tr -d '\r'), Android $("$ADB" shell getprop ro.build.version.release | tr -d '\r')"

"$ADB" install -r "$APK" >/dev/null || { echo "install failed"; exit 1; }
"$ADB" shell am force-stop $PKG
"$ADB" logcat -c

took=$("$ADB" shell am start -W -n $ACTIVITY | tr -d '\r' | awk -F': ' '/TotalTime/ {print $2}')
say INFO "cold start: ${took:-?} ms"
sleep 4

pid=$("$ADB" shell pidof $PKG | tr -d '\r')
[[ -n "$pid" ]] && say PASS "running (pid $pid)" || fail "not running after start"

# The spike measured 54-58 % of a core here, from redrawing every frame.
cpu=$("$ADB" shell top -b -n 2 -d 3 -p "$pid" 2>/dev/null | tr -d '\r' | awk -v p="$pid" '$1 == p {c = $9} END {print c}')
if [[ -n "$cpu" ]] && awk -v c="$cpu" 'BEGIN {exit !(c < 10)}'; then
  say PASS "idle on the start page: ${cpu} % CPU"
else
  fail "idle on the start page: ${cpu:-?} % CPU (want under 10)"
fi

# Back from the start page leaves the app, as any Android app does.
"$ADB" shell input keyevent KEYCODE_BACK
sleep 2
if "$ADB" shell dumpsys activity activities | tr -d '\r' | grep -q "ResumedActivity.*$PKG"; then
  fail "Back on the start page did not leave the app"
else
  say PASS "Back on the start page leaves the app"
fi

if "$ADB" logcat -d -s nimadir:E AndroidRuntime:E | grep -qi panic; then
  fail "a panic was logged:"
  "$ADB" logcat -d -s nimadir:E | grep -i -A3 panic | head -20
else
  say PASS "no panic in the log"
fi

echo
[[ $FAIL -eq 0 ]] && echo "smoke test passed" || { echo "smoke test FAILED"; exit 1; }
