#!/bin/bash
# Xcode 26 on macOS 26 workaround: Swift driver injects an absolute path to
# libswift*.dylib inside the Metal cryptex, but Metal.xctoolchain ships no
# usr/lib/ directory. Rewrite such paths to the equivalent XcodeDefault
# location before exec'ing the real clang linker.
set -e
REAL_CLANG="/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang"
GOOD_DIR="/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.0/macosx"
args=()
for a in "$@"; do
  case "$a" in
    /var/run/com.apple.security.cryptexd/mnt/*/Metal.xctoolchain/usr/lib/swift-5.0/macosx/*)
      # Swift driver on Xcode 26 injects this back-deploy path even for arm64
      # targets where it is unnecessary and the file does not exist. Drop it.
      ;;
    *) args+=("$a") ;;
  esac
done
exec "$REAL_CLANG" "${args[@]}"
