#!/usr/bin/env bash
# Build an unsigned Debug Telegram.app for macOS 26 arm64.
#
# Wraps the Xcode 26 Metal-cryptex ld-shim workaround documented in
# docs/XCODE26-BLOCKER.md. Produces a launchable Telegram.app in a
# dedicated DerivedData directory, independent of Xcode GUI state.
#
# Usage:
#   ./scripts/build-dev.sh           # build only
#   ./scripts/build-dev.sh --run     # build, then launch the app
#   ./scripts/build-dev.sh --dmg     # build, then package Telegram-seoyu.dmg

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.."; pwd)"
cd "$ROOT"

DD="$HOME/Library/Developer/Xcode/DerivedData/Telegram-Mac-dev"
APP="$DD/Build/Products/Debug/Telegram.app"

RUN=0
DMG=0
for arg in "$@"; do
  case "$arg" in
    --run) RUN=1 ;;
    --dmg) DMG=1 ;;
    *) echo "unknown flag: $arg" >&2; exit 2 ;;
  esac
done

echo "==> Ensuring submodules are initialized"
git submodule update --init --recursive >/dev/null

echo "==> Fixing shallow Firebase/GoogleAppMeasurement frameworks"
./scripts/fix-shallow-frameworks.sh

echo "==> Building Telegram.app (this takes 15–30 min on first run)"
xcodebuild build \
  -workspace Telegram-Mac.xcworkspace \
  -scheme Telegram \
  -configuration Debug \
  -destination 'generic/platform=macOS' \
  ARCHS=arm64 \
  ONLY_ACTIVE_ARCH=YES \
  CODE_SIGNING_ALLOWED=NO \
  LD="$ROOT/scripts/ld-cryptex-shim.sh" \
  LDPLUSPLUS="$ROOT/scripts/ld-cryptex-shim.sh" \
  -derivedDataPath "$DD"

test -d "$APP" || { echo "Build reported success but $APP is missing"; exit 1; }

echo "==> Built: $APP"
stat -f "    size  %z bytes    mtime %Sm" "$APP/Contents/MacOS/Telegram.debug.dylib"

if [[ "$DMG" == 1 ]]; then
  OUT="$ROOT/dist/Telegram-seoyu.dmg"
  mkdir -p "$ROOT/dist"
  rm -f "$OUT"
  hdiutil create -volname "Telegram Seoyu" -srcfolder "$APP" -ov -format UDZO "$OUT" >/dev/null
  echo "==> Packaged: $OUT  ($(du -h "$OUT" | awk '{print $1}'))"
  shasum -a 256 "$OUT" | awk '{print "    sha256 " $1}'
fi

if [[ "$RUN" == 1 ]]; then
  DB="$HOME/Library/Application Support/telegram-korean-search/tg-korean-search.db"
  if [[ -f "$DB" ]]; then
    mkdir -p "$ROOT/.backup"
    TS="$(date +%Y%m%d-%H%M%S)"
    BACKUP="$ROOT/.backup/tg-korean-search.$TS.db"
    # Use sqlite3 .backup so WAL/SHM are checkpointed into the snapshot.
    # Falls back to cp + cp -p of -wal/-shm if sqlite3 unavailable.
    if command -v sqlite3 >/dev/null 2>&1; then
      sqlite3 "$DB" ".backup '$BACKUP'"
    else
      cp -p "$DB" "$BACKUP"
      [[ -f "$DB-wal" ]] && cp -p "$DB-wal" "$BACKUP-wal"
      [[ -f "$DB-shm" ]] && cp -p "$DB-shm" "$BACKUP-shm"
    fi
    echo "==> Backed up live Seoyu DB before launch: $BACKUP"
  fi

  echo "==> Launching"
  open "$APP"
fi
