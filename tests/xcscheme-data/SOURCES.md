# `.xcscheme` test fixtures

Real-world `.xcscheme` files from open-source iOS/macOS projects, included
verbatim under their original licenses. They exist to exercise the
`src/common/xcode/xcscheme.ts` parser/serializer against the actual shapes
Xcode emits in the wild — sanitizers, RTL/locale overrides, test plans,
shell-script PreActions, etc. — rather than synthetic examples.

| Fixture | Source | License | Commit |
| --- | --- | --- | --- |
| `alamofire-ios.xcscheme` | [`Alamofire.xcodeproj/xcshareddata/xcschemes/Alamofire iOS.xcscheme`](https://github.com/Alamofire/Alamofire/blob/7595cbcf59809f9977c5f6378500de2ad73b7ddb/Alamofire.xcodeproj/xcshareddata/xcschemes/Alamofire%20iOS.xcscheme) | MIT | `7595cbc` |
| `duckduckgo-ios-browser.xcscheme` | [`DuckDuckGo-iOS.xcodeproj/xcshareddata/xcschemes/iOS Browser.xcscheme`](https://github.com/duckduckgo/iOS/blob/7b3f6010d27a3a69fe92a2ad698543f2e67c8900/DuckDuckGo-iOS.xcodeproj/xcshareddata/xcschemes/iOS%20Browser.xcscheme) | Apache-2.0 | `7b3f601` |
| `firefox-ios-redux.xcscheme` | [`BrowserKit/.swiftpm/xcode/xcshareddata/xcschemes/Redux.xcscheme`](https://github.com/mozilla-mobile/firefox-ios/blob/0d0be059832d49c205b669cd684f137ac49de107/BrowserKit/.swiftpm/xcode/xcshareddata/xcschemes/Redux.xcscheme) | MPL-2.0 | `0d0be05` |
| `kickstarter-ios.xcscheme` | [`Kickstarter.xcodeproj/xcshareddata/xcschemes/Kickstarter iOS.xcscheme`](https://github.com/kickstarter/ios-oss/blob/198fb49375d44fa6c08f2f1a8d6d9ce6de256cdd/Kickstarter.xcodeproj/xcshareddata/xcschemes/Kickstarter%20iOS.xcscheme) | Apache-2.0 | `198fb49` |
| `parse-sdk-ios.xcscheme` | [`Parse/Parse.xcodeproj/xcshareddata/xcschemes/Parse-iOS.xcscheme`](https://github.com/parse-community/Parse-SDK-iOS-OSX/blob/5a812217f2cb91f8393295f67cad859209d428a2/Parse/Parse.xcodeproj/xcshareddata/xcschemes/Parse-iOS.xcscheme) | BSD-3-Clause | `5a81221` |
| `realm-swift.xcscheme` | [`Realm.xcodeproj/xcshareddata/xcschemes/Realm.xcscheme`](https://github.com/realm/realm-swift/blob/1cd09f1a41e7336f3c3eea76b60bc979bbdf46a9/Realm.xcodeproj/xcshareddata/xcschemes/Realm.xcscheme) | Apache-2.0 | `1cd09f1` |
| `sentry-cocoa.xcscheme` | [`Sentry.xcodeproj/xcshareddata/xcschemes/Sentry.xcscheme`](https://github.com/getsentry/sentry-cocoa/blob/7d833a3f33fdafcab0102cd1312e83e1e7cc526c/Sentry.xcodeproj/xcshareddata/xcschemes/Sentry.xcscheme) | MIT | `7d833a3` |
| `signal-ios.xcscheme` | [`Signal.xcodeproj/xcshareddata/xcschemes/Signal.xcscheme`](https://github.com/signalapp/Signal-iOS/blob/4811eef51928479b9171cae3786905bbaac18a1e/Signal.xcodeproj/xcshareddata/xcschemes/Signal.xcscheme) | AGPL-3.0 | `4811eef` |
| `signal-ios-staging.xcscheme` | [`Signal.xcodeproj/xcshareddata/xcschemes/Signal-Staging.xcscheme`](https://github.com/signalapp/Signal-iOS/blob/4811eef51928479b9171cae3786905bbaac18a1e/Signal.xcodeproj/xcshareddata/xcschemes/Signal-Staging.xcscheme) | AGPL-3.0 | `4811eef` |
| `spotify-xcmetrics-basicapp.xcscheme` | [`Examples/BasicApp/BasicApp.xcodeproj/xcshareddata/xcschemes/BasicApp.xcscheme`](https://github.com/spotify/XCMetrics/blob/e1a728a2ca046d8b35a1c4b4f7e04c6758705322/Examples/BasicApp/BasicApp.xcodeproj/xcshareddata/xcschemes/BasicApp.xcscheme) | Apache-2.0 | `e1a728a` |
| `wikipedia-ios.xcscheme` | [`Wikipedia.xcodeproj/xcshareddata/xcschemes/Wikipedia.xcscheme`](https://github.com/wikimedia/wikipedia-ios/blob/806cf41d4040fda9cbefda9f319f96f11772d51f/Wikipedia.xcodeproj/xcshareddata/xcschemes/Wikipedia.xcscheme) | MIT | `806cf41` |
| `wikipedia-ios-rtl.xcscheme` | [`Wikipedia.xcodeproj/xcshareddata/xcschemes/RTL.xcscheme`](https://github.com/wikimedia/wikipedia-ios/blob/806cf41d4040fda9cbefda9f319f96f11772d51f/Wikipedia.xcodeproj/xcshareddata/xcschemes/RTL.xcscheme) | MIT | `806cf41` |
| `xcodegen-app-ios-production.xcscheme` | [`Tests/Fixtures/TestProject/Project.xcodeproj/xcshareddata/xcschemes/App_iOS Production.xcscheme`](https://github.com/yonaskolb/XcodeGen/blob/8d3d3476a69ae3e5d68e1adccc701c410c05eb36/Tests/Fixtures/TestProject/Project.xcodeproj/xcshareddata/xcschemes/App_iOS%20Production.xcscheme) | MIT | `8d3d347` |
| `xcodegen-framework.xcscheme` | [`Tests/Fixtures/TestProject/Project.xcodeproj/xcshareddata/xcschemes/Framework.xcscheme`](https://github.com/yonaskolb/XcodeGen/blob/8d3d3476a69ae3e5d68e1adccc701c410c05eb36/Tests/Fixtures/TestProject/Project.xcodeproj/xcshareddata/xcschemes/Framework.xcscheme) | MIT | `8d3d347` |

Notable coverage per fixture:
- `wikipedia-ios-rtl` — `LaunchAction.language="he"` + `region="IL"`, command-line args including `-AppleLanguages (he)`, `MacroExpansion` in `ProfileAction`, large `SkippedTests` list (the discussion-#197 use case).
- `duckduckgo-ios-browser` — `EnvironmentVariables`, `LocationScenarioReference`, multiple `TestableReference`s with cross-package `BuildableReference`s.
- `alamofire-ios` — `TestPlans` / `TestPlanReference`, `AdditionalOptions`, `LaunchAction` for a framework (`MacroExpansion` instead of `BuildableProductRunnable`).
- `parse-sdk-ios` — `enableAddressSanitizer`, `enableUBSanitizer`, `enableASanStackUseAfterReturn` on `TestAction`.
- `realm-swift` — `PreActions` / `ExecutionAction` / `ActionContent` shell script with XML-entity-escaped `scriptText` (`&quot;`, `&#10;`).
- `spotify-xcmetrics-basicapp` — `PostActions` with shell script and `runPostActionsOnFailure`.
- `kickstarter-ios` / `signal-ios` — filename with embedded space (URL-encoded as `%20`); large production app schemes.
- `xcodegen-app-ios-production` / `xcodegen-framework` — generated by [XcodeGen](https://github.com/yonaskolb/XcodeGen); child order differs from what the Xcode UI emits (`Testables` before `CommandLineArguments`), which is why `xcscheme.ts` preserves the parsed `__childOrder__` rather than imposing a canonical one.
- `firefox-ios-redux` — SPM (`.swiftpm`) scheme; minimal shape, useful sanity baseline.
