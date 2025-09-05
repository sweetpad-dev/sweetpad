load("@rules_xcodeproj//xcodeproj:defs.bzl", "top_level_target", "xcodeproj", "xcschemes")
load("//bazel_support/config:config.bzl", "xcode_configurations")
load(":common.bzl", "doordash_appclip_scheme", "doordash_scheme", "pre_build_script")
load(":generated.bzl", "schemes", "targets")

xcodeproj(
    name = "xcodeproj",
    pre_build = pre_build_script,
    project_name = "DoorDash",
    top_level_targets = [
        top_level_target(
            "//Apps/Consumer/ConsumerApp:DoorDash",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp/Tests:DoorDashTests",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp:DoorDashRed",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp:Caviar",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp/AppClip:DoorDashAppClip",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp/AppClip:DoorDashRedAppClip",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp/AppClip:CaviarAppClip",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp/LegoDevApp",
            target_environments = ["simulator"],
        ),
        top_level_target(
            "//Apps/Consumer/ConsumerApp/OverlayDevApp",
            target_environments = ["simulator"],
        ),
    ] + targets,
    xcode_configurations = xcode_configurations,
    xcschemes = [
        doordash_scheme(
            name = "DoorDash",
            run_env = {},
        ),
        doordash_scheme(
            name = "DoorDashProxy",
            run_env = {
                "PROXY_HOST": "localhost",
                "PROXY_PORT": "8888",
            },
        ),
        doordash_appclip_scheme(
            name = "DoorDashAppClip",
        ),
        doordash_appclip_scheme(
            name = "DoorDashRedAppClip",
        ),
        doordash_appclip_scheme(
            name = "CaviarAppClip",
        ),
        xcschemes.scheme(
            name = "LegoDevApp",
            run = xcschemes.run(
                build_targets = [
                    "//Apps/Consumer/ConsumerApp/LegoDevApp",
                ],
                env = {
                    "PROXY_HOST": "localhost",
                    "PROXY_PORT": "8888",
                },
                launch_target = "//Apps/Consumer/ConsumerApp/LegoDevApp",
            ),
        ),
        xcschemes.scheme(
            name = "OverlayDevApp",
            run = xcschemes.run(
                build_targets = [
                    "//Apps/Consumer/ConsumerApp/OverlayDevApp",
                ],
                env = {
                    "PROXY_HOST": "localhost",
                    "PROXY_PORT": "8888",
                },
                launch_target = "//Apps/Consumer/ConsumerApp/OverlayDevApp",
            ),
        ),
    ] + schemes,
)

# Create a dedicated xcodeproj that only builds the DoorDash for warming the Bazel cache on CI.
# This is important because we need xcodeproj to apply the same transformations to the project
# on CI as the main xcodeproj does for Xcode, but without building every target on CI.
xcodeproj(
    name = "xcodeproj_cx_warming",
    project_name = "DoorDash",
    top_level_targets = [
        top_level_target(
            "//Apps/Consumer/ConsumerApp:DoorDash",
            target_environments = ["simulator"],
        ),
    ],
)
