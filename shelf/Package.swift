// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "shelf",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(name: "Shelf",    path: "Sources/Shelf"),
        .executableTarget(name: "ShelfAdd", path: "Sources/ShelfAdd"),
    ]
)
