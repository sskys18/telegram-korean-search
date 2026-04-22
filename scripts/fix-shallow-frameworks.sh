#!/bin/bash
# macOS on Xcode 26+ enforces versioned framework bundle layout. Firebase
# and GoogleAppMeasurement ship their macos-* xcframework slices as shallow
# (iOS-style) bundles. Convert them in place to versioned layout.
set -euo pipefail

convert() {
  local fw="$1"
  local name
  name="$(basename "$fw" .framework)"

  if [[ -d "$fw/Versions" ]]; then
    return 0  # already versioned
  fi
  if [[ ! -f "$fw/Info.plist" ]]; then
    return 0  # unexpected layout, skip
  fi

  local tmp="$fw.tmp"
  rm -rf "$tmp"
  mkdir -p "$tmp/Versions/A"

  # Move everything into Versions/A, Resources/ for Info.plist
  mkdir -p "$tmp/Versions/A/Resources"
  cp "$fw/Info.plist" "$tmp/Versions/A/Resources/Info.plist"

  if [[ -f "$fw/$name" ]]; then
    cp "$fw/$name" "$tmp/Versions/A/$name"
  fi
  for sub in Headers Modules PrivateHeaders _CodeSignature; do
    if [[ -e "$fw/$sub" ]]; then
      cp -R "$fw/$sub" "$tmp/Versions/A/$sub"
    fi
  done

  ln -s "A" "$tmp/Versions/Current"
  ln -s "Versions/Current/$name" "$tmp/$name"
  [[ -e "$fw/Headers" ]] && ln -s "Versions/Current/Headers" "$tmp/Headers"
  [[ -e "$fw/Modules" ]] && ln -s "Versions/Current/Modules" "$tmp/Modules"
  ln -s "Versions/Current/Resources" "$tmp/Resources"

  rm -rf "$fw"
  mv "$tmp" "$fw"
  echo "converted $fw"
}

ROOT="$HOME/Library/Developer/Xcode/DerivedData"
while IFS= read -r fw; do
  convert "$fw"
done < <(/usr/bin/find "$ROOT" -type d -path "*/macos-*/FirebaseAnalytics.framework" -o -type d -path "*/macos-*/GoogleAppMeasurement.framework" -o -type d -path "*/macos-*/GoogleAppMeasurementIdentitySupport.framework")
