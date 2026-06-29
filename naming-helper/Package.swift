// swift-tools-version: 6.0
import PackageDescription

// On-device naming helper for herdr-plugin-renamer. Imports Apple's
// FoundationModels framework, which requires the macOS 26 (Tahoe) SDK floor.
// Flooring the platform here means the FoundationModels symbols are reachable
// without per-call @available annotations; runtime gating is done via
// SystemLanguageModel.default.availability instead.
let package = Package(
    name: "herdr-namer",
    platforms: [
        .macOS("26.0"),
    ],
    targets: [
        .executableTarget(
            name: "herdr-namer"
        ),
    ]
)
