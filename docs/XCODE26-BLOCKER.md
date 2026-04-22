# Xcode 26 build blocker — RESOLVED 2026-04-22

Previously thought to be a wait-on-Apple blocker. Unblocked by a three-part
local workaround that preserves the full TelegramSwift fork and requires no
OS/Xcode downgrade.

## Symptom (original)

Any Swift-linking target failed at the `Ld` step with:

```
clang: error: no such file or directory:
  '/var/run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-v17.5.188.0.60LGSr/Metal.xctoolchain/usr/lib/swift-5.0/macosx/libswiftAppKit.dylib'
```

## Actual root cause (revised)

The Swift driver on Xcode 26.4.1 injects an explicit back-deploy path to
Swift 5.0 standard library dylibs for AppKit, Foundation etc. The path it
picks routes through the Metal toolchain cryptex mount, which ships with no
`usr/lib/` at all (only `usr/bin/`, `usr/metal/`, `usr/share/`). On arm64
macOS 13+, that back-deploy path is not actually required — the relevant
dylibs live in the system. But the driver emits the reference anyway, and
the linker fails because the file does not exist on disk.

This is an Apple-side Swift driver bug. A local fix is to remove the bad
argument before invoking the linker.

## The fix (three parts)

### 1. Linker shim — `scripts/ld-cryptex-shim.sh`

Wrapper invoked in place of `clang` as the linker. Strips any argument
matching `…/Metal.xctoolchain/usr/lib/swift-5.0/macosx/*` from the
command line and execs the real clang. Wired in via xcodebuild:

```
LD=$(pwd)/scripts/ld-cryptex-shim.sh
LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh
```

### 2. Shallow-framework fixup — `scripts/fix-shallow-frameworks.sh`

macOS 26 enforces versioned framework bundles. The `macos-*` slices of
`FirebaseAnalytics.xcframework`, `GoogleAppMeasurement.xcframework`, and
`GoogleAppMeasurementIdentitySupport.xcframework` ship as iOS-style
shallow bundles. The script converts them to versioned layout in place,
targeting the DerivedData `SourcePackages/artifacts/…` copies that SPM
materializes. Must run before each build.

### 3. Package.swift deploy-target bump

24 `submodules/telegram-ios/submodules/*/Package.swift` manifests were
still at `.macOS(.v10_13)`. Bumped all to `.macOS(.v12)` to match the
rest of the tree. Swift-tools-version 5.5 remains unchanged.

### 4. OpenH264 staging

`core-xprojects/OpenH264/build/arm64/libopenh264.a` is produced by the
framework build but the `Telegram.xcodeproj` references it at
`core-xprojects/OpenH264/build/output/lib/libopenh264.a`. Copy the
static library into the expected location once.

## Build command

```
./scripts/fix-shallow-frameworks.sh
xcodebuild build -workspace Telegram-Mac.xcworkspace -scheme Telegram \
  -configuration Debug -destination 'generic/platform=macOS' \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
  LD=$(pwd)/scripts/ld-cryptex-shim.sh \
  LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh
```

Result on this box (Mac arm64, macOS 26.4.1, Xcode 26.4.1): `BUILD
SUCCEEDED`. Telegram.app launches.

## Things that did NOT work

- `MACOSX_DEPLOYMENT_TARGET = 13.0` in the .xcodeproj, `.macOS(.v12)` in
  every Package.swift, strict arch `ARCHS=arm64` — none of these changed
  the driver's injection behaviour.
- `TOOLCHAINS=com.apple.dt.toolchain.XcodeDefault` — skirts the cryptex
  path but then breaks the Metal shader compile step because `metal` itself
  lives inside the Metal.xctoolchain.
- `VALIDATE_WORKSPACE=NO VALIDATE_PRODUCT=NO VALIDATE_DEPLOYMENT_PLATFORM=NO`
  — does not suppress the shallow-framework check.
- Replacing the cryptex path with the XcodeDefault back-deploy path — that
  dylib is x86_64-only; the linker rejects it on arm64. Dropping the
  argument is the correct move.

## When Apple fixes this upstream

Once the Swift driver stops emitting the Metal cryptex path, the `LD=`
override can be dropped. The shallow-framework fixup remains Firebase's
responsibility — keep the script until they ship a versioned macOS slice.

The `archive/tauri-v0` tag still points at the pre-fork state of the
project if you ever need to roll back to the Tauri companion app.
