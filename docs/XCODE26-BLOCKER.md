# Xcode 26 build blocker

## Symptom

Any Swift-linking target (TelegramShare, Telegram, FocusIntents) fails at the
`Ld` step with:

```
clang: error: no such file or directory:
  '/var/run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-v17.5.188.0.60LGSr/Metal.xctoolchain/usr/lib/swift-5.0/macosx/libswiftAppKit.dylib'
```

## Root cause

Xcode 26.4.1 ships on macOS 26 (Tahoe) with the Metal toolchain bundled as a
read-only APFS cryptex mount at
`/var/run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-…/Metal.xctoolchain`.

The Swift driver's back-compat stdlib lookup routes to
`<active-toolchain>/usr/lib/swift-5.0/macosx/` whenever a target deploys below
macOS 10.14.4. In Xcode 26.4.1 the Metal toolchain is selected as the
"active toolchain" for this lookup, but that bundle ships no
`usr/lib/` directory at all (only `usr/bin/`, `usr/metal/`, `usr/share/`). The
linker is invoked with an absolute filename that physically does not exist
on disk.

Verified:

```
$ ls /var/run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-*/Metal.xctoolchain/usr/lib
ls: .../Metal.xctoolchain/usr/lib: No such file or directory
```

The cryptex is mounted read-only with an APFS seal (SSV), so a symlink
patch inside the cryptex is impossible even with root.

## Things that do NOT fix it

Every combination of the following was tried on this project and produced
the same error:

- `MACOSX_DEPLOYMENT_TARGET = 13.0` in the .xcodeproj
- `.macOS(.v12)` / `.v13` in every Package.swift in tree
- `ARCHS=arm64 ONLY_ACTIVE_ARCH=YES` (skip x86_64 slice)
- `-configuration Release` (debug vs release does not matter)
- `EAGER_LINKING=NO EAGER_LINKING_REQUIRES_EMIT_TBD=NO`
- `-toolchain com.apple.dt.toolchain.XcodeDefault` (breaks Metal shader
  compilation, same root cause)
- `SWIFT_USE_INTEGRATED_DRIVER=NO`
- `SWIFT_ENABLE_EXPLICIT_MODULES=NO`
- `sudo xcodebuild -runFirstLaunch`
- `xcodebuild -downloadComponent MetalToolchain` (already run)
- Fresh DerivedData + fresh SPM cache
- Building only the `Telegram` target via `-project` (still pulls the
  extension targets as embed deps)

## Paths forward

1. **Wait for Apple**. File at https://feedbackassistant.apple.com referencing
   the exact path + the Metal toolchain asset id
   `com.apple.MobileAsset.MetalToolchain-v17.5.188.0.60LGSr`. Check each Xcode
   point release.

2. **Install Xcode 16.x** *only possible on macOS ≤ 15*. Tahoe will refuse
   to launch older Xcode. Requires downgrading the OS or dual-booting a
   Sequoia partition.

3. **Pivot to an overlay client** built with `swift build` + Command Line
   Tools only. Bypasses the whole .xcodeproj / cryptex pipeline. Keeps the
   Rust sidecar. Ships a menu-bar app with a Cmd+Shift+F Korean-search
   overlay that deep-links into the official Telegram Desktop.

4. **Write a TDLib-based SwiftUI client from scratch** with `swift build`.
   Works (no .xcodeproj = no cryptex bug) but is a ~3-month solo project
   for even a text-only MVP.

See the conversation handoff for which of these the project is currently
pursuing.

## What upstream edits did land

Several upstream-level fixes are already committed because they are correct
regardless of which path forward we take:

- `core-xprojects/ffmpeg/ffmpeg/build.sh` — pin `FF_VERSION=7.1.1`
- `core-xprojects/webrtc/webrtc/build.sh` — add `-Wno-error` to cmake flags
- `configurations/*.xcconfig` — strip `APPCENTER_SECRET` and `SFEED_URL`
- `Telegram.xcodeproj/project.pbxproj` — bump deployment target to 13.0,
  rename bundle ids to `com.seoyu.telegram-seoyu`
- `packages/*/Package.swift` and `submodules/telegram-ios/submodules/*/Package.swift`
  — bump `.macOS(.v10_*)` to `.macOS(.v12)`
- `packages/ApiCredentials/Sources/ApiCredentials/Config.swift.example` —
  template; real `Config.swift` is gitignored and contains developer credentials
- 981 Bazel `BUILD` files deleted from `submodules/telegram-ios/` (APFS
  case-insensitive clash with Xcode's `build/` intermediates dir; not
  checked in since they live inside a submodule and would be restored on
  the next `git submodule update`)

The `archive/tauri-v0` tag still points at the pre-fork state of the
project if you want to roll back to the Tauri companion app.
