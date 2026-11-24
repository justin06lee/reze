#!/bin/bash
# Build the phone app, put it on a simulated iPhone and take its picture.
#
#   ios/sim.sh [mac-name-or-address]     (make ios-sim MAC=mac-mini)
#
# With a Mac named, the app is started already knowing which Mac to fetch
# from, so what comes out is its real library on a real iOS. A simulated phone
# is on this Mac's own network, so it reaches a Tailscale address the way a
# phone on the tailnet would.
#
# Nothing opens on screen: the simulator is started without its window. To
# use it by hand afterwards, `open -a Simulator`.
set -euo pipefail
cd "$(dirname "$0")/.."

MAC="${1:-}"
NAME="Reze iPhone"
APP_ID="com.justin06lee.reze.ios"
BUILD="ios/build"
SHOTS="ios/shots"

xcodebuild -version >/dev/null 2>&1 || {
  echo "No usable Xcode. Select one (sudo xcode-select -s /Applications/Xcode.app, sudo xcodebuild -license)"
  echo "or name one for this run: DEVELOPER_DIR=/path/to/Xcode.app/Contents/Developer make ios-sim"
  exit 1
}
command -v xcodegen >/dev/null || { echo "xcodegen is not installed: brew install xcodegen"; exit 1; }

echo "[1/5] writing the Xcode project"
xcodegen generate --spec ios/project.yml >/dev/null

echo "[2/5] building the app and its keyboard for the simulator (a minute or two the first time)"
log="$(mktemp)"
if ! xcodebuild -project ios/Reze.xcodeproj -scheme Reze -configuration Debug \
    -destination 'generic/platform=iOS Simulator' -derivedDataPath "$BUILD" build >"$log" 2>&1; then
  grep -E 'error:|BUILD FAILED' "$log" | head -40
  echo "The build failed. The whole log: $log"
  exit 1
fi
rm -f "$log"

echo "[3/5] starting a simulated iPhone"
device="$(xcrun simctl list devices available | sed -n "s/^ *$NAME (\([0-9A-F-]*\)).*/\1/p" | head -1)"
if [ -z "$device" ]; then
  runtime="$(xcrun simctl list runtimes | sed -n 's/^iOS .* - \(com\.apple\.CoreSimulator\.SimRuntime\.iOS-[0-9-]*\)$/\1/p' | tail -1)"
  [ -n "$runtime" ] || { echo "This Xcode has no iOS simulator installed (Xcode → Settings → Components)."; exit 1; }
  # The newest plain iPhone this runtime will take: the list of models also
  # names ones newer than the runtime, which it refuses.
  for model in $(xcrun simctl list devicetypes | sed -n 's/^iPhone \([0-9][0-9]*\) (.*/\1/p' | sort -rn); do
    device="$(xcrun simctl create "$NAME" "iPhone $model" "$runtime" 2>/dev/null)" && break
  done
  [ -n "$device" ] || { echo "Could not make a simulated iPhone for $runtime."; exit 1; }
fi
xcrun simctl boot "$device" 2>/dev/null || true
xcrun simctl bootstatus "$device" -b >/dev/null

echo "[4/5] installing and opening Reze${MAC:+, to fetch from $MAC}"
xcrun simctl terminate "$device" "$APP_ID" >/dev/null 2>&1 || true
xcrun simctl install "$device" "$BUILD/Build/Products/Debug-iphonesimulator/Reze.app"
if [ -n "$MAC" ]; then
  xcrun simctl launch "$device" "$APP_ID" -RezeMac "$MAC" >/dev/null
else
  xcrun simctl launch "$device" "$APP_ID" >/dev/null
fi

echo "[5/5] taking its picture"
mkdir -p "$SHOTS"
# Long enough to fetch a library and draw it.
sleep 6
xcrun simctl io "$device" screenshot "$SHOTS/simulator.png" >/dev/null 2>&1
echo "Done: $SHOTS/simulator.png"

if [ -n "$MAC" ]; then
  kept="$(xcrun simctl get_app_container "$device" "$APP_ID" group.com.justin06lee.reze 2>/dev/null)/Reze"
  if [ -f "$kept/library.json" ]; then
    echo "On the simulated phone: $(plutil -extract macros raw -o - "$kept/library.json") macros," \
         "$(ls "$kept/assets" 2>/dev/null | wc -l | tr -d ' ') attachment files."
  else
    echo "Nothing was fetched from $MAC. The picture says why."
  fi
fi
echo "The simulator is still running, without a window. See it: open -a Simulator. Stop it: xcrun simctl shutdown \"$NAME\""
