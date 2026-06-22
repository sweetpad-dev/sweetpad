---
title: Copying Shared Cache Symbols in Xcode
---

# Understanding Xcode's "Copying Shared Cache Symbols" Process

When you work with Xcode, you have probably seen this message appear during builds or when connecting devices:
**"Copying Shared Cache Symbols."** This process can take several minutes, especially the first time you connect a new
device or update iOS. Understanding what happens during this step will help you work more efficiently with Xcode and
debug your applications better.

<!-- truncate -->

## What Are Symbols?

Before we explore shared cache symbols, let's understand what symbols are in software development. Symbols are pieces of
information that connect your compiled code back to the original source code. They include function names, variable
names, file names, and line numbers.

When you compile Swift code, the compiler creates machine code that the processor can execute. This machine code
consists of memory addresses and instructions, but it does not contain the original function names or variable names
from your source code. Symbols bridge this gap.

Here is a simple example. Consider this Swift function:

```swift
func calculateTotal(price: Double, tax: Double) -> Double {
    let total = price + (price * tax)
    return total
}
```

After compilation, this becomes machine code with memory addresses. Without symbols, a debugger would show something
like:

```
0x100001f20: add x0, x1, x2
0x100001f24: mul x3, x1, x4
0x100001f28: ret
```

With symbols, the debugger can show:

```
calculateTotal(price:tax:) at ViewController.swift:15
    let total = price + (price * tax)
```

This makes debugging much easier because you can see your actual code instead of raw memory addresses.

## What Is the Shared Cache?

iOS uses a shared cache system to optimize memory usage and app launch times. The shared cache contains pre-compiled
versions of system frameworks like UIKit, Foundation, Core Data, and many others. Instead of each app loading its own
copy of these frameworks, all apps share the same cached versions.

This shared cache lives in the device's system partition. When your app uses UIKit to create a button, it uses the
shared cache version of UIKit rather than including its own copy. This saves significant memory and makes apps launch
faster.

The shared cache gets updated with each iOS version. When Apple releases iOS 17.1, the shared cache contains optimized
versions of all system frameworks compiled specifically for that iOS version.

## Why Does Xcode Copy Shared Cache Symbols?

When you debug an app on a device, you often step through code that calls system frameworks. For example, when you set a
breakpoint in this code:

```swift
override func viewDidLoad() {
    super.viewDidLoad() // This calls UIKit code
    setupUI()
}
```

The debugger needs to show you what happens inside `super.viewDidLoad()`. This method is part of UIKit, which lives in
the shared cache on your device. To display meaningful information about UIKit functions, Xcode needs the symbols for
the shared cache.

However, your development Mac does not have these symbols by default. Your Mac runs macOS, while your device runs iOS.
The shared cache symbols on your device are specific to that iOS version and device architecture.

Therefore, Xcode must copy the shared cache symbols from your device to your Mac. This allows the debugger to show you
readable information when stepping through system framework code.

## When Does This Process Happen?

Xcode copies shared cache symbols in several situations:

**First Device Connection**: When you connect a new device for the first time, Xcode needs to get the symbols for that
device's iOS version.

**iOS Updates**: When you update your device to a new iOS version, the shared cache changes. Xcode needs the new symbols
that match the updated frameworks.

**Xcode Updates**: When you update Xcode, it might need fresh copies of symbols to work with its improved debugging
tools.

**Manual Symbol Copying**: You can trigger this process manually through Xcode's Window menu by selecting "Devices and
Simulators" and choosing your device.

## What Happens During Symbol Copying?

The copying process involves several steps that happen automatically:

**Connection Establishment**: Xcode connects to your device through USB or Wi-Fi and establishes a secure communication
channel.

**Version Detection**: Xcode checks the iOS version and device architecture to determine which symbols it needs.

**Symbol Extraction**: Xcode reads the shared cache from your device and extracts symbol information. This includes
function names, class names, and debugging information for all system frameworks.

**Local Storage**: Xcode stores these symbols in a special directory on your Mac, typically at:

```
~/Library/Developer/Xcode/iOS DeviceSupport/[iOS Version] [Device Architecture]/Symbols/
```

**Index Building**: Xcode builds an index of these symbols so the debugger can quickly find the information it needs
during debugging sessions.

## How This Affects Your Development Workflow

Understanding this process helps you work more efficiently:

**Debugging System Code**: After symbol copying completes, you can set breakpoints in system framework code and see
meaningful stack traces. For example, if your app crashes in a UIKit method, you will see the actual method names
instead of memory addresses.

**Performance Profiling**: Instruments and other profiling tools can show you time spent in system frameworks with
proper function names, making it easier to identify performance bottlenecks.

**Crash Analysis**: When your app crashes and generates a crash report, the symbols allow you to see exactly which
system methods were involved in the crash.

Here is an example of how symbols improve crash reports. Without symbols, you might see:

```
0   libsystem_kernel.dylib    0x00000001c234567 0x1c2340000 + 1656
1   UIKitCore                 0x00000001a789012 0x1a7890000 + 122
```

With symbols, the same crash appears as:

```
0   libsystem_kernel.dylib    objc_exception_throw + 56
1   UIKitCore                 -[UIView addSubview:] + 122
```

This makes it immediately clear that the crash happened when adding a subview to a UIView.

## Storage and Management

Xcode stores copied symbols in your user directory, and these files can become quite large over time. Each iOS version
for each device architecture creates its own symbol directory. An iPhone 15 running iOS 17.1 will have different symbols
than an iPad running iOS 16.4.

You can manage these symbols through Xcode's preferences. If you find that symbol directories are taking up too much
disk space, you can delete symbol folders for iOS versions or devices you no longer use. Xcode will re-copy symbols when
needed.

## Common Issues and Solutions

Sometimes the symbol copying process encounters problems:

**Slow Copying**: Large shared caches can take time to copy, especially over Wi-Fi connections. Using a USB connection
typically speeds up the process.

**Interrupted Copying**: If copying gets interrupted, Xcode might have incomplete symbols. Delete the partial symbol
directory and reconnect your device to restart the process.

**Missing Symbols**: If debugging shows memory addresses instead of function names, the symbol copying might have
failed. Check the Devices and Simulators window for error messages.

## Conclusion

The "Copying Shared Cache Symbols" process is essential for effective iOS development. It enables proper debugging,
profiling, and crash analysis by providing readable information about system framework code. While this process takes
time initially, it significantly improves your development experience by making debugging sessions more informative and
crash reports more actionable.

Understanding this process helps you plan your development workflow better. When you get a new device or update iOS,
expect to spend a few extra minutes on symbol copying. This upfront investment pays off through improved debugging
capabilities throughout your development cycle.

:::info

This article is written with the help of AI, because I am not proficient in writing English. If you find any mistakes or
have suggestions for improvement, please [open an issue](https://github.com/sweetpad-dev/sweetpad-docs/issues) on
GitHub. Thanks for your understanding!

---

Yevhenii Hyzyla, SweetPad Developer

:::
