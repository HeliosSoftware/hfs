// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "HFSClient",
    platforms: [
        .iOS(.v17),
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "HFSClientKit",
            targets: [
                "HFSCore",
                "HFSFHIR",
                "HFSHTTP",
                "HFSAuth",
                "HFSClient",
                "HFSOperations"
            ]
        ),
        .library(
            name: "HFSAdminUI",
            targets: ["HFSAdminUI"]
        ),
        .executable(
            name: "hfs-admin",
            targets: ["HFSAdminApp"]
        )
    ],
    targets: [
        .target(name: "HFSCore"),
        .target(name: "HFSFHIR"),
        .target(
            name: "HFSHTTP",
            dependencies: ["HFSCore"]
        ),
        .target(
            name: "HFSAuth",
            dependencies: ["HFSCore"]
        ),
        .target(
            name: "HFSClient",
            dependencies: [
                "HFSCore",
                "HFSFHIR",
                "HFSHTTP",
                "HFSAuth"
            ]
        ),
        .target(
            name: "HFSOperations",
            dependencies: [
                "HFSCore",
                "HFSFHIR",
                "HFSClient"
            ]
        ),
        .target(
            name: "HFSAdminUI",
            dependencies: [
                "HFSCore",
                "HFSFHIR",
                "HFSHTTP",
                "HFSClient",
                "HFSOperations"
            ]
        ),
        .executableTarget(
            name: "HFSAdminApp",
            dependencies: ["HFSAdminUI"]
        ),
        .testTarget(
            name: "HFSCoreTests",
            dependencies: ["HFSCore"]
        ),
        .testTarget(
            name: "HFSFHIRTests",
            dependencies: ["HFSFHIR"]
        ),
        .testTarget(
            name: "HFSClientTests",
            dependencies: ["HFSClient"]
        ),
        .testTarget(
            name: "HFSOperationsTests",
            dependencies: [
                "HFSOperations",
                "HFSClient",
                "HFSHTTP",
                "HFSCore"
            ]
        ),
        .testTarget(
            name: "HFSAdminUITests",
            dependencies: [
                "HFSAdminUI",
                "HFSCore",
                "HFSHTTP"
            ]
        )
    ]
)
