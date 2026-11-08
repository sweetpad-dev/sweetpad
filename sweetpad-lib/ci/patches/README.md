# Corpus compatibility patches

These patches prepare pinned coverage fixtures for the current Xcode and Swift toolchains.
They are explicit setup steps. Sweetpad does not patch application sources during a build.
Source revisions and patch order are in `../linux-corpus-patches.json`.

After package resolution, apply the corpus patches on the Mac:

```sh
python3 -B sweetpad-lib/ci/linux-corpus-patches.py
```

The VM shares corpus sources with the Mac. Its resolved package checkouts are separate:

```sh
orb -m sweetpad-linux python3 -B \
  "$PWD/sweetpad-lib/ci/linux-corpus-patches.py" --resolved-only
```

Use `--derived-data PATH` for another Ice Cubes package cache.
Use `--skip-resolved` before package resolution. Use `--check` to validate without writing.
The script checks every source revision and accepts patches that are already applied.

| Fixture | Patch purpose |
| --- | --- |
| Ice Cubes Models | Qualify `SwiftSoup.Document` to resolve the SDK name collision. |
| Ice Cubes value isolation | Mark two value-only helpers and the immutable report response `nonisolated`. UI helpers retain their actor isolation. |
| Ice Cubes EnvTests | Add an explicit test-only scheme, equivalent to its other package test schemes. |
| Ice Cubes TimelineTests | Run the stateful mock on the main actor, as its protocol requires. |
| Ice Cubes tag editor | Move the warning's computed property outside the view builder, so its use follows a valid declaration. |
| RevenueCat | Move the private designated initializer into the primary type. This suppresses the conflicting synthesized initializer. |
| NetNewsWire | Make the outer closure's existing strong capture explicit. Keep the inner weak capture. |
| NetNewsWire keychain query | Remove return-value and match-limit options from `SecItemDelete`. Both app-hosted controls pass all 92 AccountTests. |
| Tuist static libraries | Declare the prebuilt C module dependency for ATests and include its Swift module search path. |
| Alamofire, Kingfisher, Tuist watch apps | Merge extension code, assets, dependencies, and framework embedding into each watch app target. |

The watch migrations preserve each app's storyboard and interface controllers.
They update storyboard modules, adopt `WKApplicationDelegate`, and add a `WKApplicationMain` entry point.
The app property lists use `WKApplication`. The deployment target is watchOS 11.
The obsolete extension products and their embedding phases are removed.
The Tuist `WatchAppExtension` scheme is retired; `WatchApp` builds the merged application.
The manifest records this retirement and Alamofire's watch simulator destination.

Native and Linux compilation validate the migrated watch apps.
Watch runtime and physical-device behavior remain unverified because those runtimes and devices are unavailable.

The `_spm-broken` fixture deliberately references a missing package. It remains an expected failure.
The additional MMKV fixture uses `react-native-mmkv-cxx-debug-thunks.patch`.
Its support-header shim suppresses forced emission of unused generic debug thunks in Linux Swift 6.4.2.
The shim preserves generated APIs and restores `DEBUG` before application headers load.
Apple's compiler retains its original behavior.
`react-native-mmkv-scene-lifecycle.patch` fixes the shared SDK 27 scene-lifecycle trap.
Apply these source patches with `linux-projects.py --only react-native-mmkv --prepare-only`.
Then regenerate the example's Pods with `bundle exec pod install` inside `example/ios`.

`swift-nio-apple-cross-resources.patch` selects privacy resources during Apple destination manifest evaluation.
Run `linux-dependency-patches.py` after package resolution on each host.
`linux-projects.json` pins the separate Mastodon and Swiftfin dependency revisions.
This preparation remains explicit. The build engine never edits package sources.

`react-native-template-scene-lifecycle.patch` and `statefulmacros-linux-foundation.patch`
belong to the additional real-project suite described in `../LINUX_PROJECTS.md`.

The original PIF snapshot tests read pristine pinned sources and reverse recorded project-only patches in temporary generated Tuist containers.
They retain legacy watch-product coverage without reverting the migrated build corpus.
