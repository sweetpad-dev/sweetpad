# SwiftUI autolink regression

`swiftui-autolink.s` places SwiftUICore before SwiftUI in Mach-O autolink commands.
Xcode links the referenced symbol through SwiftUI. LLD 21.1.6 selects SwiftUICore first.
Xcode keeps the reference through its public SwiftUI umbrella at the Wikipedia fixture's iOS 17.6 deployment target.
The linker wrapper prepends an empty object with a SwiftUI autolink command when both framework options occur.
This keeps the umbrella implicit, so unused frameworks are still removed.
Explicit weak-framework policy and unloaded archives retain their original behavior.
The generated object remains in the user's state directory so linker dependency files stay valid.

`sweetpad-lib/ci/linux-linking-checks.py` compares native, raw LLD, and wrapped outputs.
It covers both framework orders, unused imports, and explicit weak imports at four deployment targets.

Regenerate the native object with the same SDK used by the app controls:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun clang \
  -target arm64-apple-ios17.6-simulator \
  -isysroot /Applications/Xcode.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk \
  -c sweetpad-cli/fixtures/linker/swiftui-autolink.s \
  -o sweetpad-cli/fixtures/linker/swiftui-autolink.bin
```

The `.bin` suffix keeps this regression fixture separate from build objects.
