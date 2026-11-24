// swift-tools-version: 5.9
import PackageDescription

// Everything the phone app and its keyboard share, kept in a package so that
// it builds and is tested on a Mac with `swift test` — no Xcode, no simulator.
//
//   RezeKit  Foundation only: the library as a Mac hands it over, the template
//            language, fuzzy matching, the client for a Mac's Reze, the cache.
//   RezeUI   SwiftUI: every screen of the app and the keyboard's panel. The
//            parts only a phone has are fenced with `#if os(iOS)`.
let package = Package(
    name: "RezeKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "RezeKit", targets: ["RezeKit"]),
        .library(name: "RezeUI", targets: ["RezeUI"]),
    ],
    targets: [
        .target(name: "RezeKit"),
        .target(name: "RezeUI", dependencies: ["RezeKit"]),
        .executableTarget(name: "Shots", dependencies: ["RezeKit", "RezeUI"]),
        .testTarget(name: "RezeKitTests", dependencies: ["RezeKit"], resources: [.copy("Fixtures")]),
        .testTarget(name: "RezeUITests", dependencies: ["RezeKit", "RezeUI"]),
    ]
)
