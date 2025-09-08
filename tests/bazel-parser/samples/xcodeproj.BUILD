load("@rules_xcodeproj//xcodeproj:defs.bzl", "top_level_target", "xcodeproj", "xcschemes")
load("//bazel_support/config:config.bzl", "xcode_configurations")
load(":common.bzl", "doordash_appclip_scheme", "doordash_scheme", "pre_build_script")
load(":generated.bzl", "schemes", "targets")

xcodeproj(
    name = "xcodeproj",
    pre_build = pre_build_script,
    project_name = "SampleApp",
    top_level_targets = [
        top_level_target(
            "//Apps/SampleApp/MainApp:SampleApp",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp/Tests:SampleAppTests",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp:SampleAppPro",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp:SampleAppLite",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp/AppClip:SampleAppClip",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp/AppClip:SampleAppProClip",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp/AppClip:SampleAppLiteClip",
            target_environments = [
                "simulator",
                "device",
            ],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp/DevApp",
            target_environments = ["simulator"],
        ),
        top_level_target(
            "//Apps/SampleApp/MainApp/TestApp",
            target_environments = ["simulator"],
        ),
    ] + targets,
    xcode_configurations = xcode_configurations,
    xcschemes = [
        doordash_scheme(
            name = "SampleApp",
            run_env = {},
        ),
        doordash_scheme(
            name = "SampleAppProxy",
            run_env = {
                "PROXY_HOST": "localhost",
                "PROXY_PORT": "8888",
            },
        ),
        doordash_appclip_scheme(
            name = "SampleAppClip",
        ),
        doordash_appclip_scheme(
            name = "SampleAppProClip",
        ),
        doordash_appclip_scheme(
            name = "SampleAppLiteClip",
        ),
        xcschemes.scheme(
            name = "DevApp",
            run = xcschemes.run(
                build_targets = [
                    "//Apps/SampleApp/MainApp/DevApp",
                ],
                env = {
                    "PROXY_HOST": "localhost",
                    "PROXY_PORT": "8888",
                },
                launch_target = "//Apps/SampleApp/MainApp/DevApp",
            ),
        ),
        xcschemes.scheme(
            name = "TestApp",
            run = xcschemes.run(
                build_targets = [
                    "//Apps/SampleApp/MainApp/TestApp",
                ],
                env = {
                    "PROXY_HOST": "localhost",
                    "PROXY_PORT": "8888",
                },
                launch_target = "//Apps/SampleApp/MainApp/TestApp",
            ),
        ),
    ] + schemes,
)

# Create a dedicated xcodeproj that only builds the SampleApp for warming the Bazel cache on CI.
# This is important because we need xcodeproj to apply the same transformations to the project
# on CI as the main xcodeproj does for Xcode, but without building every target on CI.
xcodeproj(
    name = "xcodeproj_cx_warming",
    project_name = "SampleApp",
    top_level_targets = [
        top_level_target(
            "//Apps/SampleApp/MainApp:SampleApp",
            target_environments = ["simulator"],
        ),
    ],
)
