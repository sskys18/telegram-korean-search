// swift-tools-version:5.9
//
// Swift Package wrapping the Rust sidecar via UniFFI.
//
// The generated Swift wrapper (`Sources/Seoyu/Generated/seoyu.swift`)
// and the C-ABI header + modulemap (`Sources/SeoyuFFI/include/`) are
// emitted by `scripts/build-seoyu-xcframework.sh`. Both are
// gitignored; the script must be run once after cloning and any time
// the sidecar's UniFFI surface changes. The Xcode build phase
// attached to Telegram-Mac calls the same script so day-to-day work
// does not involve running it manually.
//
// `SeoyuFFI.xcframework` is likewise generated. It bundles the
// universal-binary static library that the Swift target ultimately
// links against.

import PackageDescription
import Foundation

// The binary target (SeoyuFFI.xcframework) is produced by
// scripts/build-seoyu-xcframework.sh and is gitignored. On a fresh
// clone the xcframework does not exist yet and `swift package
// describe` would reject a hard binaryTarget declaration. This
// manifest therefore declares the Swift target unconditionally and
// only pulls in the binary target once the artifact is present, so
// the package parses cleanly at every stage of the developer's
// workflow.

let xcframeworkPath = "SeoyuFFI.xcframework"
let manifestDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let hasXcframework = FileManager.default.fileExists(
    atPath: manifestDir.appendingPathComponent(xcframeworkPath).path
)

// The generated `Generated/seoyu.swift` imports the SeoyuFFI
// module, which only exists once the xcframework is present. When
// it is missing we exclude that directory so the package still
// compiles (with only the placeholder) and the developer sees a
// clean error telling them to run the build script.
var targets: [Target] = [
    .target(
        name: "Seoyu",
        dependencies: hasXcframework ? ["SeoyuFFI"] : [],
        path: "Sources/Seoyu",
        exclude: hasXcframework ? [] : ["Generated"]
    ),
]
if hasXcframework {
    targets.append(.binaryTarget(name: "SeoyuFFI", path: xcframeworkPath))
}

let package = Package(
    name: "Seoyu",
    platforms: [.macOS(.v12)],
    products: [
        .library(name: "Seoyu", targets: ["Seoyu"])
    ],
    targets: targets
)
