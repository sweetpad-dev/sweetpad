load("//bazel_support/rules:dd_ios_package.bzl", "dd_ios_package", "target")

dd_ios_package(
    name = "ExampleSDK",
    targets = [
        target.library(
            name = "ExampleCaching",
            deps = [
                ":ExampleDataStructures",
            ],
            path = "Sources/Caching",
        ),
        target.library(
            name = "ExampleCombineExtensions",
            deps = [
                ":ExampleCommandExecutor",
                ":ExampleDomainLogic",
            ],
            path = "Sources/CombineExtensions",
        ),
        target.library(
            name = "ExampleCommandExecutor",
            deps = [
                ":ExampleThreading",
            ],
            path = "Sources/CommandExecutor",
        ),
        target.library(
            name = "ExampleDataStructures",
            deps = [
                "@swiftpkg_swift_collections//:OrderedCollections",
            ],
            path = "Sources/DataStructures",
        ),
        target.library(
            name = "ExampleDomainLogic",
            deps = [
                ":ExampleCommandExecutor",
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
            ],
            path = "Sources/DomainLogic",
        ),
        target.library(
            name = "ExampleFoundationExtensions",
            path = "Sources/FoundationExtensions",
        ),
        target.library(
            name = "ExampleKeychain",
            path = "Sources/Keychain",
        ),
        target.library(
            name = "ExampleLinkHandling",
            deps = [
                ":ExampleNetworking",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/LinkHandling",
        ),
        target.library(
            name = "ExampleLocation",
            deps = [
                ":ExampleDataStructures",
                ":ExampleFoundationExtensions",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Location",
        ),
        target.library(
            name = "ExampleLogging",
            deps = [
                ":ExampleThreading",
            ],
            path = "Sources/Logging",
        ),
        target.library(
            name = "ExampleMath",
            deps = [
                ":ExampleDataStructures",
                ":ExampleFoundationExtensions",
            ],
            path = "Sources/Math",
        ),
        target.library(
            name = "ExampleNetworking",
            deps = [
                ":ExampleFoundationExtensions",
                ":ExampleLogging",
                ":ExampleSerialization",
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Networking",
        ),
        target.library(
            name = "ExampleNotificationSupport",
            deps = [
                ":ExampleNetworking",
            ],
            path = "Sources/NotificationSupport",
        ),
        target.library(
            name = "ExamplePresentation",
            deps = [
                ":ExampleCommandExecutor",
                ":ExampleDataStructures",
                ":ExampleDomainLogic",
                ":ExampleNetworking",
                ":ExampleThreading",
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
            ],
            path = "Sources/Presentation",
        ),
        target.library(
            name = "ExampleSerialization",
            deps = [
                ":ExampleFoundationExtensions",
                ":ExampleSwiftUIExtensions",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Serialization",
        ),
        target.library(
            name = "ExampleSwiftUIExtensions",
            path = "Sources/SwiftUIExtensions",
        ),
        target.library(
            name = "ExampleThreading",
            deps = [
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Threading",
        ),
        target.library(
            name = "ExampleUnitTestHelpers",
            deps = [
                ":ExampleCaching",
                ":ExamplePresentation",
            ],
            path = "Sources/UnitTestHelpers",
            resources = [
                "Sources/UnitTestHelpers/Resources/Images/testimage.png",
                "Sources/UnitTestHelpers/Resources/Images/image1.png",
                "Sources/UnitTestHelpers/Resources/Images/image2.png",
                "Sources/UnitTestHelpers/Resources/Images/image3.png",
            ],
        ),
        target.test(
            name = "AsyncCommandExecutorTests",
            deps = [
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/AsyncCommandExecutorTests",
        ),
        target.test(
            name = "ExampleCachingTests",
            deps = [
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/CachingTests",
        ),
        target.test(
            name = "ExampleCommandExecutorTests",
            deps = [
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/CommandExecutorTests",
        ),
        target.test(
            name = "ExampleDataStructuresTests",
            deps = [
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/DataStructuresTests",
        ),
        target.test(
            name = "ExampleDomainLogicTests",
            deps = [
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/DomainLogicTests",
        ),
        target.test(
            name = "ExampleFoundationExtensionsTests",
            deps = [
                ":ExampleFoundationExtensions",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/FoundationExtensionsTests",
        ),
        target.test(
            name = "ExampleLocationTests",
            deps = [
                ":ExampleLocation",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/LocationTests",
        ),
        target.test(
            name = "ExampleLoggingTests",
            deps = [
                ":ExampleLogging",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/LoggingTests",
        ),
        target.test(
            name = "ExampleMathTests",
            deps = [
                ":ExampleMath",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/MathTests",
        ),
        target.test(
            name = "ExampleNetworkingTests",
            deps = [
                ":ExampleNetworking",
                ":ExampleSerialization",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/NetworkingTests",
        ),
        target.test(
            name = "ExampleNotificationSupportTests",
            deps = [
                ":ExampleNotificationSupport",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/NotificationSupportTests",
        ),
        target.test(
            name = "ExamplePresentationTests",
            deps = [
                ":ExampleDomainLogic",
                ":ExamplePresentation",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/PresentationTests",
        ),
        target.test(
            name = "ExampleSerializationTests",
            deps = [
                ":ExampleSerialization",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/SerializationTests",
        ),
        target.test(
            name = "ExampleThreadingTests",
            deps = [
                ":ExampleThreading",
                ":ExampleUnitTestHelpers",
            ],
            path = "Tests/ThreadingTests",
        ),
    ],
)
