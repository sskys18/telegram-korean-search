#!/usr/bin/env bash
#
# Builds the Seoyu static libraries for both macOS architectures,
# lipo-merges them into a universal binary, emits Swift bindings
# from the dylib, and wraps everything in an .xcframework that the
# Swift app target at packages/Seoyu/ can depend on.
#
# Run this once after a fresh clone and any time sidecar/ source
# changes. The Xcode build phase attached to the Telegram-Mac scheme
# calls this same script so day-to-day the developer never invokes
# it directly.
#
# Requirements:
#   - rustup targets aarch64-apple-darwin and x86_64-apple-darwin
#   - xcodebuild (ships with full Xcode, not just Command Line Tools)
#   - lipo (ships with Command Line Tools)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIDECAR_DIR="${REPO_ROOT}/sidecar"
PACKAGE_DIR="${REPO_ROOT}/packages/Seoyu"
XCFRAMEWORK_OUT="${PACKAGE_DIR}/SeoyuFFI.xcframework"
BINDINGS_OUT="${PACKAGE_DIR}/Sources/Seoyu/Generated"

PROFILE="${PROFILE:-release}"
CARGO_FLAGS=(--manifest-path "${SIDECAR_DIR}/Cargo.toml" --lib)
if [[ "${PROFILE}" == "release" ]]; then
    CARGO_FLAGS+=(--release)
fi

TARGET_ARM="aarch64-apple-darwin"
TARGET_X86="x86_64-apple-darwin"
TARGET_DIR_ARM="${SIDECAR_DIR}/target/${TARGET_ARM}/${PROFILE}"
TARGET_DIR_X86="${SIDECAR_DIR}/target/${TARGET_X86}/${PROFILE}"
UNIVERSAL_DIR="${SIDECAR_DIR}/target/universal-apple-darwin/${PROFILE}"

echo "[1/5] Building Rust staticlib for ${TARGET_ARM}"
cargo build --target "${TARGET_ARM}" "${CARGO_FLAGS[@]}"

echo "[2/5] Building Rust staticlib for ${TARGET_X86}"
cargo build --target "${TARGET_X86}" "${CARGO_FLAGS[@]}"

echo "[3/5] lipo-merging into universal static lib"
mkdir -p "${UNIVERSAL_DIR}"
lipo -create \
    "${TARGET_DIR_ARM}/libseoyu.a" \
    "${TARGET_DIR_X86}/libseoyu.a" \
    -output "${UNIVERSAL_DIR}/libseoyu.a"

echo "[4/5] Generating Swift bindings"
mkdir -p "${BINDINGS_OUT}"
# uniffi-bindgen shells out to `cargo metadata` from its own cwd
# to find the crate; run it from the sidecar dir so that resolves.
(
    cd "${SIDECAR_DIR}"
    cargo run --bin uniffi-bindgen -- \
        generate \
        --library "target/${TARGET_ARM}/${PROFILE}/libseoyu.dylib" \
        --language swift \
        --out-dir "${BINDINGS_OUT}"
)

# UniFFI emits the Swift wrapper, a C header, and a modulemap. Move
# the C header + modulemap under a module-qualified include dir so
# Swift Package Manager can find them as a system-library target.
MODULE_DIR="${PACKAGE_DIR}/Sources/SeoyuFFI/include"
mkdir -p "${MODULE_DIR}"
mv "${BINDINGS_OUT}/seoyuFFI.h" "${MODULE_DIR}/seoyuFFI.h"
mv "${BINDINGS_OUT}/seoyuFFI.modulemap" "${MODULE_DIR}/module.modulemap"

echo "[5/5] Assembling xcframework"
rm -rf "${XCFRAMEWORK_OUT}"
xcodebuild -create-xcframework \
    -library "${UNIVERSAL_DIR}/libseoyu.a" \
    -headers "${MODULE_DIR}" \
    -output "${XCFRAMEWORK_OUT}"

echo ""
echo "Done. Output:"
echo "  ${XCFRAMEWORK_OUT}"
echo "  ${BINDINGS_OUT}/seoyu.swift"
echo ""
echo "Open Telegram-Mac.xcworkspace and ensure the Seoyu Swift Package"
echo "at ${PACKAGE_DIR} is listed as a package dependency of the"
echo "Telegram-Mac app target."
