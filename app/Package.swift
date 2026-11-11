// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "ThermoPilot",
    platforms: [.macOS(.v15)],
    targets: [
        .executableTarget(name: "ThermoPilot", path: "Sources/ThermoPilot")
    ],
    swiftLanguageModes: [.v5]
)
