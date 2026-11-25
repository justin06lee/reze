#!/bin/bash
# The phone app to TestFlight (internal testing): a signed Release archive,
# exported for the App Store and uploaded to App Store Connect.
#
#   ios/testflight.sh            archive, upload, and wait until it can be installed
#   UPLOAD=0 ios/testflight.sh   archive and export only (ios/build/export/Reze.ipa)
#   BUILD=7 ios/testflight.sh    with that build number, not the time
#
# Needs an Xcode (the selected one, or DEVELOPER_DIR), xcodegen, bun, the App
# Store Connect API key in ~/.appstoreconnect/private_keys, and the team's
# Apple Distribution identity in the signing keychain. `bun ios/asc.ts check`
# says what else is missing; two of those things only a person can do.
#
# Signing is manual on purpose. Xcode's own automatic signing is refused with
# this API key ("Authentication failed … bearer token"), and Apple's cloud
# signing is refused to its role, so the profiles are made through the API
# (ios/asc.ts) and named here.
set -euo pipefail
cd "$(dirname "$0")/.."

TEAM="${TEAM_ID:-65UN46K6FA}"
KEY_ID="${ASC_KEY_ID:-Y688NH26RQ}"
ISSUER="${ASC_ISSUER_ID:-fd10fb1e-e9c4-4570-bda4-e93f2e10c8ff}"
KEY="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$KEY_ID.p8}"
KEYCHAIN="${SIGNING_KEYCHAIN:-$HOME/Library/Keychains/midna-signing.keychain-db}"
BUILD="${BUILD:-$(date +%Y%m%d%H%M)}"
UPLOAD="${UPLOAD:-1}"
OUT=ios/build
ARCHIVE="$OUT/Reze.xcarchive"

xcodebuild -version >/dev/null 2>&1 || {
  echo "No usable Xcode. Select one, or name one: DEVELOPER_DIR=/path/to/Xcode.app/Contents/Developer make ios-testflight"
  exit 1
}
[ -f "$KEY" ] || { echo "No App Store Connect API key at $KEY"; exit 1; }

# Run a step, keeping what it prints out of the way unless it fails.
quietly() {
  local log="$1"; shift
  if ! "$@" >"$log" 2>&1; then
    grep -E 'error:|\*\* .* FAILED|rror ' "$log" | head -30 || true
    echo "Failed. The whole log: $log"
    exit 1
  fi
}

echo "[1/6] the identifiers, the signing key and the profiles"
bun ios/asc.ts ids
# The keychain's own password is kept in the login keychain by whoever made
# it; with it, codesign never stops to ask.
if secret="$(security find-generic-password -s midna-signing-keychain -w 2>/dev/null)"; then
  security unlock-keychain -p "$secret" "$KEYCHAIN"
fi
IDENTITY="$(bun ios/asc.ts identity)"
bun ios/asc.ts profiles

if [ "$UPLOAD" = 1 ]; then
  echo "[2/6] the app's record in App Store Connect"
  bun ios/asc.ts check
else
  echo "[2/6] not uploading, so the app's record is not looked for"
fi

echo "[3/6] writing the Xcode project"
mkdir -p "$OUT"
xcodegen generate --spec ios/project.yml >/dev/null
rm -rf "$ARCHIVE" "$OUT/export"

echo "[4/6] archiving build $BUILD (a few minutes)"
quietly "$OUT/archive.log" xcodebuild -project ios/Reze.xcodeproj -scheme Reze -configuration Release \
  -destination generic/platform=iOS -derivedDataPath "$OUT/device" -archivePath "$ARCHIVE" \
  DEVELOPMENT_TEAM="$TEAM" CURRENT_PROJECT_VERSION="$BUILD" \
  CODE_SIGN_IDENTITY="$IDENTITY" OTHER_CODE_SIGN_FLAGS="--keychain $KEYCHAIN" archive

destination=export; [ "$UPLOAD" = 1 ] && destination=upload
cat > "$OUT/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>$destination</string>
  <key>teamID</key><string>$TEAM</string>
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>$IDENTITY</string>
  <key>provisioningProfiles</key>
  <dict>
    <key>com.justin06lee.reze.ios</key><string>Reze App Store</string>
    <key>com.justin06lee.reze.ios.keyboard</key><string>Reze Keyboard App Store</string>
  </dict>
  <key>manageAppVersionAndBuildNumber</key><false/>
  <key>uploadSymbols</key><true/>
  <key>testFlightInternalTestingOnly</key><true/>
</dict>
</plist>
PLIST

if [ "$UPLOAD" = 1 ]; then echo "[5/6] uploading build $BUILD"; else echo "[5/6] exporting build $BUILD"; fi
quietly "$OUT/export.log" xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportPath "$OUT/export" \
  -exportOptionsPlist "$OUT/ExportOptions.plist" \
  -allowProvisioningUpdates -authenticationKeyPath "$KEY" -authenticationKeyID "$KEY_ID" -authenticationKeyIssuerID "$ISSUER"

if [ "$UPLOAD" != 1 ]; then
  echo "[6/6] done: $(ls "$OUT"/export/*.ipa)"
  exit 0
fi

echo "[6/6] testers, and waiting for App Store Connect to process it (a few minutes)"
bun ios/asc.ts testers
bun ios/asc.ts wait "$BUILD"
echo "Build $BUILD is on TestFlight. On the phone: open the TestFlight app and install Reze."
