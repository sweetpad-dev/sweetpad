load("//bazel_support/rules:dd_ios_package.bzl", "dd_ios_package", "target")

dd_ios_package(
    name = "Topaz",
    targets = [
        target.library(
            name = "TopazCaching",
            deps = [
                ":TopazDataStructures",
            ],
            path = "Sources/Caching",
        ),
        target.library(
            name = "TopazCombineExtensions",
            deps = [
                ":TopazCommandExecutor",
                ":TopazDomainLogic",
            ],
            path = "Sources/CombineExtensions",
        ),
        target.library(
            name = "TopazCommandExecutor",
            deps = [
                ":TopazThreading",
            ],
            path = "Sources/CommandExecutor",
        ),
        target.library(
            name = "TopazDataStructures",
            deps = [
                "@swiftpkg_swift_collections//:OrderedCollections",
            ],
            path = "Sources/DataStructures",
        ),
        target.library(
            name = "TopazDomainLogic",
            deps = [
                ":TopazCommandExecutor",
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
            ],
            path = "Sources/DomainLogic",
        ),
        target.library(
            name = "TopazFoundationExtensions",
            path = "Sources/FoundationExtensions",
        ),
        target.library(
            name = "TopazKeychain",
            path = "Sources/Keychain",
        ),
        target.library(
            name = "TopazLinkHandling",
            deps = [
                ":TopazNetworking",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/LinkHandling",
        ),
        target.library(
            name = "TopazLocation",
            deps = [
                ":TopazDataStructures",
                ":TopazFoundationExtensions",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Location",
        ),
        target.library(
            name = "TopazLogging",
            deps = [
                ":TopazThreading",
            ],
            path = "Sources/Logging",
        ),
        target.library(
            name = "TopazMath",
            deps = [
                ":TopazDataStructures",
                ":TopazFoundationExtensions",
            ],
            path = "Sources/Math",
        ),
        target.library(
            name = "TopazNetworking",
            deps = [
                ":TopazFoundationExtensions",
                ":TopazLogging",
                ":TopazSerialization",
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Networking",
        ),
        target.library(
            name = "TopazNotificationSupport",
            deps = [
                ":TopazNetworking",
            ],
            path = "Sources/NotificationSupport",
        ),
        target.library(
            name = "TopazPresentation",
            deps = [
                ":TopazCommandExecutor",
                ":TopazDataStructures",
                ":TopazDomainLogic",
                ":TopazNetworking",
                ":TopazThreading",
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
            ],
            path = "Sources/Presentation",
        ),
        target.library(
            name = "TopazSerialization",
            deps = [
                ":TopazFoundationExtensions",
                ":TopazSwiftUIExtensions",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Serialization",
        ),
        target.library(
            name = "TopazSwiftUIExtensions",
            path = "Sources/SwiftUIExtensions",
        ),
        target.library(
            name = "TopazThreading",
            deps = [
                "@swiftpkg_combine_schedulers//:CombineSchedulers",
                "@swiftpkg_swift_dependencies//:Dependencies",
            ],
            path = "Sources/Threading",
        ),
        target.library(
            name = "TopazUnitTestHelpers",
            deps = [
                ":TopazCaching",
                ":TopazPresentation",
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
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/AsyncCommandExecutorTests",
        ),
        target.test(
            name = "TopazCachingTests",
            deps = [
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/CachingTests",
        ),
        target.test(
            name = "TopazCommandExecutorTests",
            deps = [
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/CommandExecutorTests",
        ),
        target.test(
            name = "TopazDataStructuresTests",
            deps = [
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/DataStructuresTests",
        ),
        target.test(
            name = "TopazDomainLogicTests",
            deps = [
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/DomainLogicTests",
        ),
        target.test(
            name = "TopazFoundationExtensionsTests",
            deps = [
                ":TopazFoundationExtensions",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/FoundationExtensionsTests",
        ),
        target.test(
            name = "TopazLocationTests",
            deps = [
                ":TopazLocation",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/LocationTests",
        ),
        target.test(
            name = "TopazLoggingTests",
            deps = [
                ":TopazLogging",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/LoggingTests",
        ),
        target.test(
            name = "TopazMathTests",
            deps = [
                ":TopazMath",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/MathTests",
        ),
        target.test(
            name = "TopazNetworkingTests",
            deps = [
                ":TopazNetworking",
                ":TopazSerialization",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/NetworkingTests",
        ),
        target.test(
            name = "TopazNotificationSupportTests",
            deps = [
                ":TopazNotificationSupport",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/NotificationSupportTests",
        ),
        target.test(
            name = "TopazPresentationTests",
            deps = [
                ":TopazDomainLogic",
                ":TopazPresentation",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/PresentationTests",
        ),
        target.test(
            name = "TopazSerializationTests",
            deps = [
                ":TopazSerialization",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/SerializationTests",
        ),
        target.test(
            name = "TopazThreadingTests",
            deps = [
                ":TopazThreading",
                ":TopazUnitTestHelpers",
            ],
            path = "Tests/ThreadingTests",
        ),
    ],
)
