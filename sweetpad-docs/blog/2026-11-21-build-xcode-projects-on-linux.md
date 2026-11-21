---
title: Build Xcode projects on Linux with SweetPad
---

import ThemedImage from '@theme/ThemedImage';

:::info About this article
SweetPad is a command-line tool for building, running, and testing Xcode projects without opening Xcode. It can also build Xcode projects without changes on **Linux**. You can build a native iOS app there and run it on a connected iPhone. This article follows one small app through that build, from the project file to the finished app bundle, and shows which tool does each step.
:::

<!-- truncate -->

On Linux, SweetPad reads the Xcode project, describes it to Swift Build, and lets Swift Build run the build. The Linux Swift compiler, Clang, and LLD produce the iPhone code against the SDK imported from Xcode. SweetPad stands in for the Xcode tools that compile resources and sign the app.

<ThemedImage
  alt="SweetPad turns the Xcode project and scheme into a build description for Swift Build. Swift Build runs swiftc, Clang, and LLD against the Apple SDK, and SweetPad’s actool, ibtool, and codesign. Both paths produce MyApp.app."
  sources={{
    light: '/images/blog/linux-build/overview-light.svg',
    dark: '/images/blog/linux-build/overview-dark.svg',
  }}
/>

To see each step, we’ll follow a small SwiftUI app called MyApp. Its first screen shows a greeting, and the message lives in Greeting.swift, an ordinary Swift file with no imports.

```swift
enum Greeting {
    static let message = "Hello from a Linux build"
}
```

ContentView.swift shows that message in a [SwiftUI view](https://developer.apple.com/documentation/swiftui/view).

```swift
import SwiftUI

struct ContentView: View {
    var body: some View {
        Text(Greeting.message)
    }
}
```

The rest of the project is the app’s entry point in MyAppApp.swift, an asset catalog, and the Xcode project that lists them all.

```text
MyApp/
├── MyApp.xcodeproj/
│   └── project.pbxproj
└── MyApp/
    ├── MyAppApp.swift
    ├── ContentView.swift
    ├── Greeting.swift
    └── Assets.xcassets/
```

Greeting.swift and ContentView.swift belong to the iPhone app target. To build them on Linux, the Swift compiler needs to know what `Text` means. Later, the iPhone needs to know how to draw it.

First, we [prepare the Apple SDK](/docs/cli/linux/sdk-transfer) on the Linux machine. Then we build MyApp from its project directory.

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp --configuration Debug \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$PWD/DerivedData" CODE_SIGNING_ALLOWED=NO > build.json
```

With `generic/platform=iOS`, the build targets an iPhone without one plugged in. The command puts the Debug build in the project’s DerivedData directory and turns off signing. It saves the JSON report to build.json so we can find the finished app later.

## How SweetPad knows what to build

SweetPad opens the [Xcode project](https://developer.apple.com/library/archive/featuredarticles/XcodeConcepts/Concept-Projects.html) passed with `--project`. Inside the .xcodeproj directory, project.pbxproj lists the files in the app target and how to build them. Greeting.swift and ContentView.swift are in the same Swift module, so the view can refer to `Greeting` by name.

`--scheme MyApp` picks the [scheme](https://developer.apple.com/documentation/xcode/customizing-the-build-schemes-for-a-project), which says which targets belong to this build. For a test run, the scheme can include the test target along with the app.

SweetPad then writes the targets and their dependencies to workspace.pif.json in Swift Build’s [Project Interchange Format (PIF)](https://github.com/swiftlang/swift-build/blob/main/SwiftBuild.docc/Core/project-interchange-format.md). If the app uses Swift packages, Swift Package Manager (SwiftPM) provides their targets too. For a standalone project like this one, SweetPad reads pinned package versions from MyApp.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved.

SweetPad writes these inputs to DerivedData/Build/Intermediates.noindex/SweetPad/. Find MyApp in workspace.pif.json, and its target ID leads to `configuredTargets` in build-request.json. Next to them, build-parameters.json records the Debug configuration and the iOS destination.

SweetPad starts [`swbuild build`](https://github.com/swiftlang/swift-build) with those files. It points Swift Build at the Apple platforms by setting `XCODE_EXTRA_PLATFORM_FOLDERS` to the Developer/Platforms directory of the SDK bundle that setup imported from Xcode. For an iOS device destination, Swift Build uses the iPhoneOS SDK in that bundle.

Swift Build is the build engine Xcode uses. It schedules the compile task and runs swiftc from the Linux Swift toolchain with the iPhone SDK and target architecture. swiftc is the program that compiles Greeting.swift and ContentView.swift.

While swbuild runs, `sweetpad build` reads its progress messages and records compiler warnings and errors. When the build succeeds, SweetPad finds the finished app and puts its path in the JSON report in build.json. When it fails, `sweetpad build diagnostics` shows the recorded errors.

## How the Swift file becomes an iPhone executable

To check `Text(Greeting.message)`, the compiler reads our source for the type of `Greeting.message`. For `Text`, it reads SwiftUI’s module interface from the iPhone software development kit (SDK), which is why setup imports the SDK from [Xcode](https://developer.apple.com/xcode/). The interface has the declarations the compiler needs to check the SwiftUI calls. The view itself appears later, when SwiftUI evaluates its body on the phone.

Even on an x86_64 Linux machine, swiftc writes ARM64 instructions for the iPhone into **object files** with the .o extension. If the target has C or Objective-C sources, Clang compiles them for the same destination.

Next, the linker joins the object files into the executable that runs on the phone. SweetPad ships a Linux build of LLVM’s linker, [LLD](https://lld.llvm.org/MachO/index.html), with support for Apple’s Mach-O executable format.

To link against SwiftUI, LLD reads the SDK’s text-based library stubs (.tbd files), which list the symbols each framework provides. The executable records a dependency on SwiftUI, and iOS loads the framework when the app launches. A screen written with [UIKit](https://developer.apple.com/documentation/uikit) goes through the same compile and link steps.

<ThemedImage
  alt="swiftc compiles Greeting.swift and ContentView.swift against the SwiftUI interface in the iPhoneOS SDK. LLD links the object files with the SDK’s framework stubs into the MyApp executable."
  sources={{
    light: '/images/blog/linux-build/compile-and-link-light.svg',
    dark: '/images/blog/linux-build/compile-and-link-dark.svg',
  }}
/>

## What else goes into the app bundle

A target’s Swift sources can include generated code alongside the files we wrote. Xcode turns on [asset symbol generation](https://developer.apple.com/documentation/xcode-release-notes/xcode-15-release-notes/#Asset-Catalogs) by default. With it on, actool writes GeneratedAssetSymbols.swift, which gives images and colors names that Swift code can use, and the Swift compiler waits for that file. A separate task compiles the same catalog into Assets.car for the app bundle.

<ThemedImage
  alt="SweetPad’s actool reads Assets.xcassets. It writes Assets.car into MyApp.app and, when symbols are enabled, GeneratedAssetSymbols.swift for swiftc to compile."
  sources={{
    light: '/images/blog/linux-build/asset-catalog-light.svg',
    dark: '/images/blog/linux-build/asset-catalog-dark.svg',
  }}
/>

If the catalog has an image named Logo, the generated symbol lets us write `Image(.logo)` in SwiftUI or `UIImage(resource: .logo)` in UIKit. That name only matters at compile time. On the device, [CoreUI](https://theapplewiki.com/wiki/CoreUI) reads the image from Assets.car inside the app bundle.

Swift Build runs actool for both catalog tasks. On Linux, actool is a link to the SweetPad executable, and SweetPad picks the tool to run from the name it was called by. The same mechanism covers the other tools Xcode would run:

- ibtool for [storyboards](https://developer.apple.com/documentation/uikit/uistoryboard) and [XIBs](https://developer.apple.com/documentation/uikit/uinib)
- momc and mapc for [Core Data models](https://developer.apple.com/documentation/coredata/modeling-data) and [mapping models](https://developer.apple.com/documentation/coredata/nsmappingmodel)
- xcstringstool for [String Catalogs](https://developer.apple.com/documentation/xcode/localizing-and-varying-text-with-a-string-catalog)
- appintentsmetadataprocessor for [App Intents](https://developer.apple.com/documentation/appintents) metadata
- codesign for [signing](/docs/cli/linux/signing-and-testing#configure-development-signing)

Some code runs on the Linux host during the build instead of on the phone. A build-tool plugin on the app target could generate Greeting.swift from a JSON file. The plugin’s generator runs on Linux, and swiftc compiles its output for iOS with the rest of the target.

Macros run inside the compiler. If we declare a property with `@Entry` in an extension of SwiftUI’s `EnvironmentValues`, the compiler asks SweetPad’s Linux macro plugin to expand it into the environment key and the accessors SwiftUI uses. A macro from a Swift package is different. SwiftPM compiles it from source for the Linux host, so its code has to build on Linux. The [support reference](/docs/cli/linux/reference#projects-and-resources) lists which macros and plugins work.

## Inside the built app

The JSON report in build.json gives the app’s path in `data.productPath`. With these build settings, the path ends in Debug-iphoneos/MyApp.app. A simulator build goes under Debug-iphonesimulator/ instead. For our example, the bundle holds three files.

```text
MyApp.app/
├── Info.plist
├── MyApp
└── PkgInfo
```

MyApp is the executable that LLD linked from Greeting.swift, ContentView.swift, and MyAppApp.swift. It is a Mach-O file, the executable format, for arm64, the processor architecture. A simulator build for Apple silicon is an arm64 Mach-O file too, so the folder name tells the two apart.

Info.plist is a binary property list that iOS reads before it launches the app. It records the bundle identifier, the minimum iOS version, and the SDK the app was built with. PkgInfo is eight bytes that mark the bundle as an application.

The bundle has no Assets.car because the example’s asset catalog holds no images or colors yet. Add a color to the catalog, and the next build puts Assets.car next to the other three files.

Xcode’s Debug build of the same project adds two more files, MyApp.debug.dylib and __preview.dylib, for its previews. The Linux build leaves them out.

## Building again

If we change the message in Greeting.swift and build again, Swift Build reuses work whose inputs and commands haven’t changed. The compiler handles the changed source, the linker makes a new executable, and the asset catalog and other resources stay as they are.

Upgrading SweetPad or importing an SDK from a different Xcode makes Swift Build compile resources again, even when the files haven’t changed. Either one changes the tools that compile them.

Two builds that share a DerivedData folder run one after the other. The second prints a note and waits for the first to finish.

## Next steps

To see the greeting on a phone, [build with development signing](/docs/cli/linux/signing-and-testing#configure-development-signing), then [install the signed app](/docs/cli/linux/ubuntu-usb#build-install-and-launch). For iOS tests, Linux can build the test bundles and write the manifest Xcode needs to [run them on a Mac](/docs/cli/linux/signing-and-testing#run-the-tests-on-a-mac).

## Apple’s license terms

The build needs the iPhone SDK from Xcode, and Apple licenses that SDK with conditions on where it runs.

:::warning Apple SDK license
Using an Apple SDK on hardware that Apple didn’t make may violate your agreement with Apple. Apple’s standard terms limit execution to Apple-branded hardware running macOS. Before you import the SDK, check that your terms permit the machine you build on. See the [Xcode and Apple SDKs Agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
:::
