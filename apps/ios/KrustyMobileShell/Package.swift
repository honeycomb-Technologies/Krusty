// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "KrustyMobileShell",
    platforms: [.iOS(.v16)],
    products: [
        .library(name: "KrustyMobileShell", targets: ["KrustyMobileShell"]),
    ],
    targets: [
        .target(name: "KrustyMobileShell"),
    ]
)
