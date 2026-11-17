#!/bin/sh
# Builds the app in Release for the simulator and fails when a test place's station id is
# anywhere in the built app (the binary, its libraries and its bundled resources).
# Run on the Mac, from the repository root:
#   cargo xtask ios remote exec -- sh ios/shots/check-release-places.sh
set -eu
derived=.derived-release
xcodebuild -quiet -project ios/Underbite/Underbite.xcodeproj -scheme Underbite \
    -configuration Release -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$derived" CODE_SIGNING_ALLOWED=NO ARCHS=arm64 ONLY_ACTIVE_ARCH=NO build
app="$derived/Build/Products/Release-iphonesimulator/Underbite.app"
[ -f "$app/Underbite" ] || { echo "no binary in $app" >&2; exit 2; }
found=0
for id in 8518750 01375000 44025; do
    if grep -rlaF "$id" "$app"; then
        echo "test place $id is in the Release build (files above)" >&2
        found=1
    fi
done
[ "$found" = 0 ] && echo "release check: none of 8518750, 01375000, 44025 in $app"
exit "$found"
