// swift-tools-version:6.2
// UnderbiteKit: the Rust core (underbite-ffi) for the iOS app.
// Build the XCFramework and bindings first: `cargo xtask ios build` (on the Mac).

import PackageDescription

let package = Package(
    name: "UnderbiteKit",
    defaultLocalization: "en",
    platforms: [.iOS(.v26), .macOS(.v26)],
    products: [
        .library(name: "UnderbiteKit", targets: ["UnderbiteKit"])
    ],
    targets: [
        .binaryTarget(name: "underbite_ffiFFI", path: "UnderbiteFFI.xcframework"),
        // Swift 5 mode: UniFFI's generated async callbacks do not pass Swift 6's data-race
        // checks yet. The tests and the app use Swift 6.
        .target(
            name: "UnderbiteKit", dependencies: ["underbite_ffiFFI"],
            resources: [.process("Resources")],
            swiftSettings: [.swiftLanguageMode(.v5)]),
        .testTarget(
            name: "UnderbiteKitTests", dependencies: ["UnderbiteKit"],
            resources: [.copy("Resources")]),
    ]
)
