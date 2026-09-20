# fixtures/FIXTURES.md

**Generated — do not hand-edit.** Capture-completeness section from `scripts/05_validate.py`, feature-probe section from `scripts/06_audit_coverage.py`; each rebuilds this file from `REPORT.json` + `AUDIT.json` after refreshing its own data. The curated coverage interpretation lives in `DOCS.md` §9.

## Capture completeness

Per (corpus project × Xcode version), a rough 0-100 score over metadata, per-scheme captures, raw inputs, and smoke builds. Synthetic fixtures (`_*`) are excluded — their layouts don't follow the corpus rubric; the probe audit below covers them. The retired `dry-run/` captures are not scored (Xcode 26 removed `-dry-run`).

| Project | xcode-15.4.0 | xcode-16.4.0 | xcode-26.5.0 | xcode-27.0.0 |
|---|---|---|---|---|
| alamofire | — | 69% | 80% | 80% |
| ice-cubes | — | 30% | 80% | 75% |
| kingfisher | 74% | 74% | 80% | 80% |
| netnewswire | — | — | 80% | 79% |
| tuist-fixtures | 66% | — | 80% | 74% |

### Per-cell detail

#### alamofire

##### xcode-16.4.0
- completeness: **69%**
- list.json: OK
- showsdks.json: OK
- raw files: 19
- schemes: 7 (Alamofire iOS, Alamofire visionOS, iOS Example, Alamofire tvOS, Alamofire watchOS, Alamofire macOS, watchOS Example WatchKit App)
- schemes with destinations.json: 7/7
- schemes with build-settings/: 2/7
- builds: total=0, exit0=0, complete_artifacts=0

##### xcode-26.5.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 19
- schemes: 7 (Alamofire iOS, Alamofire visionOS, iOS Example, Alamofire tvOS, Alamofire watchOS, Alamofire macOS, watchOS Example WatchKit App)
- schemes with destinations.json: 7/7
- schemes with build-settings/: 7/7
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__watchOS-Example-WatchKit-App__Debug__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_buildsettings__watchOS-Example-WatchKit-App__Release__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_list__root.txt`

##### xcode-27.0.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 19
- schemes: 7 (Alamofire iOS, Alamofire visionOS, iOS Example, Alamofire tvOS, Alamofire watchOS, Alamofire macOS, watchOS Example WatchKit App)
- schemes with destinations.json: 7/7
- schemes with build-settings/: 7/7
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__watchOS-Example-WatchKit-App__Debug__iOS-Simulator_OS27.0_iPhone-17.txt`
  - `02_buildsettings__watchOS-Example-WatchKit-App__Release__iOS-Simulator_OS27.0_iPhone-17.txt`

#### ice-cubes

##### xcode-16.4.0
- completeness: **30%**
- list.json: MISSING
- showsdks.json: OK
- raw files: 19
- schemes: 0 ()
- schemes with destinations.json: 0/0
- schemes with build-settings/: 0/0
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_list__root.txt`

##### xcode-26.5.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 19
- schemes: 25 (MediaUI, DesignSystem, IceCubesApp, StatusKit-Package, AccountTests, IceCubesAppWidgetsExtensionExtension, TimelineTests, Lists…)
- schemes with destinations.json: 25/25
- schemes with build-settings/: 25/25
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__AccountTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Debug__macOS.txt`
  - `02_buildsettings__AccountTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__AccountTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__AccountTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Release__macOS.txt`
  - `02_buildsettings__AccountTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__AccountTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__EnvTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__EnvTests__Debug__macOS.txt`
  - `02_buildsettings__EnvTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__EnvTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__EnvTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__EnvTests__Release__macOS.txt`
  - `02_buildsettings__EnvTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__EnvTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__ModelsTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__ModelsTests__Debug__macOS.txt`
  - `02_buildsettings__ModelsTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__ModelsTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__ModelsTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__ModelsTests__Release__macOS.txt`
  - `02_buildsettings__ModelsTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__ModelsTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__StatusKitTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__StatusKitTests__Debug__macOS.txt`
  - `02_buildsettings__StatusKitTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__StatusKitTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__StatusKitTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__StatusKitTests__Release__macOS.txt`
  - `02_buildsettings__StatusKitTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__StatusKitTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__TimelineTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__TimelineTests__Debug__macOS.txt`
  - `02_buildsettings__TimelineTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__TimelineTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__TimelineTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__TimelineTests__Release__macOS.txt`
  - `02_buildsettings__TimelineTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__TimelineTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_destinations__NetworkTests.txt`
  - `02_destinations__RevenueCatUITests.txt`
  - `02_list__root.txt`

##### xcode-27.0.0
- completeness: **75%**
- list.json: OK
- showsdks.json: OK
- raw files: 19
- schemes: 27 (MediaUI, DesignSystem, IceCubesApp, RevenueCatUITests, NetworkTests, StatusKit-Package, AccountTests, IceCubesAppWidgetsExtensionExtension…)
- schemes with destinations.json: 25/27
- schemes with build-settings/: 20/27
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__AccountTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Debug__macOS.txt`
  - `02_buildsettings__AccountTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__AccountTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__AccountTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__AccountTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Release__macOS.txt`
  - `02_buildsettings__AccountTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__AccountTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__AccountTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__EnvTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__EnvTests__Debug__macOS.txt`
  - `02_buildsettings__EnvTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__EnvTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__EnvTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__EnvTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__EnvTests__Release__macOS.txt`
  - `02_buildsettings__EnvTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__EnvTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__EnvTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__ModelsTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__ModelsTests__Debug__macOS.txt`
  - `02_buildsettings__ModelsTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__ModelsTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__ModelsTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__ModelsTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__ModelsTests__Release__macOS.txt`
  - `02_buildsettings__ModelsTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__ModelsTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__ModelsTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__StatusKitTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__StatusKitTests__Debug__macOS.txt`
  - `02_buildsettings__StatusKitTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__StatusKitTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__StatusKitTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__StatusKitTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__StatusKitTests__Release__macOS.txt`
  - `02_buildsettings__StatusKitTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__StatusKitTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__StatusKitTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__TimelineTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__TimelineTests__Debug__macOS.txt`
  - `02_buildsettings__TimelineTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__TimelineTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__TimelineTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__TimelineTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__TimelineTests__Release__macOS.txt`
  - `02_buildsettings__TimelineTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__TimelineTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__TimelineTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_destinations__NetworkTests.txt`
  - `02_destinations__RevenueCatUITests.txt`

#### kingfisher

##### xcode-15.4.0
- completeness: **74%**
- list.json: OK
- showsdks.json: OK
- raw files: 16
- schemes: 5 (Kingfisher-watchOS-Demo, Kingfisher-macOS-Demo, Kingfisher-Demo, Kingfisher-tvOS-Demo, Kingfisher)
- schemes with destinations.json: 5/5
- schemes with build-settings/: 3/5
- builds: total=0, exit0=0, complete_artifacts=0

##### xcode-16.4.0
- completeness: **74%**
- list.json: OK
- showsdks.json: OK
- raw files: 16
- schemes: 5 (Kingfisher-watchOS-Demo, Kingfisher-macOS-Demo, Kingfisher-Demo, Kingfisher-tvOS-Demo, Kingfisher)
- schemes with destinations.json: 5/5
- schemes with build-settings/: 3/5
- builds: total=0, exit0=0, complete_artifacts=0

##### xcode-26.5.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 16
- schemes: 5 (Kingfisher-watchOS-Demo, Kingfisher-macOS-Demo, Kingfisher-Demo, Kingfisher-tvOS-Demo, Kingfisher)
- schemes with destinations.json: 5/5
- schemes with build-settings/: 5/5
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__Kingfisher-watchOS-Demo__Debug__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_buildsettings__Kingfisher-watchOS-Demo__Release__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_list__root.txt`

##### xcode-27.0.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 16
- schemes: 5 (Kingfisher-watchOS-Demo, Kingfisher-macOS-Demo, Kingfisher-Demo, Kingfisher-tvOS-Demo, Kingfisher)
- schemes with destinations.json: 5/5
- schemes with build-settings/: 5/5
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__Kingfisher-watchOS-Demo__Debug__iOS-Simulator_OS27.0_iPhone-17.txt`
  - `02_buildsettings__Kingfisher-watchOS-Demo__Release__iOS-Simulator_OS27.0_iPhone-17.txt`

#### netnewswire

##### xcode-26.5.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 57
- schemes: 25 (FeedFinder, RSTree, AccountTests, RSCore, CloudKitSync, Secrets, RSParser, ArticlesDatabase…)
- schemes with destinations.json: 25/25
- schemes with build-settings/: 25/25
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__AccountTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Debug__macOS.txt`
  - `02_buildsettings__AccountTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__AccountTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__AccountTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Release__macOS.txt`
  - `02_buildsettings__AccountTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__AccountTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__RSWebTests__Debug__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__RSWebTests__Debug__macOS.txt`
  - `02_buildsettings__RSWebTests__Debug__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__RSWebTests__Debug__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__RSWebTests__Release__iOS-Simulator_OS26.5_iPad-A16.txt`
  - `02_buildsettings__RSWebTests__Release__macOS.txt`
  - `02_buildsettings__RSWebTests__Release__tvOS-Simulator_OS26.5_Apple-TV.txt`
  - `02_buildsettings__RSWebTests__Release__watchOS-Simulator_OS26.5_Apple-Watch-SE-3-40mm.txt`
  - `02_list__root.txt`

##### xcode-27.0.0
- completeness: **79%**
- list.json: OK
- showsdks.json: OK
- raw files: 57
- schemes: 25 (FeedFinder, RSTree, AccountTests, RSCore, CloudKitSync, Secrets, RSParser, ArticlesDatabase…)
- schemes with destinations.json: 25/25
- schemes with build-settings/: 23/25
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__AccountTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Debug__macOS.txt`
  - `02_buildsettings__AccountTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__AccountTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__AccountTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__AccountTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__AccountTests__Release__macOS.txt`
  - `02_buildsettings__AccountTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__AccountTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__AccountTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__ArticlesDatabase__Release__macOS.txt`
  - `02_buildsettings__RSWebTests__Debug__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__RSWebTests__Debug__macOS.txt`
  - `02_buildsettings__RSWebTests__Debug__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__RSWebTests__Debug__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__RSWebTests__Debug__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`
  - `02_buildsettings__RSWebTests__Release__iOS-Simulator_OS27.0_iPad-A16.txt`
  - `02_buildsettings__RSWebTests__Release__macOS.txt`
  - `02_buildsettings__RSWebTests__Release__tvOS-Simulator_OS27.0_Apple-TV-4K-3rd-generation.txt`
  - `02_buildsettings__RSWebTests__Release__visionOS-Simulator_OS27.0_Apple-Vision-Pro.txt`
  - `02_buildsettings__RSWebTests__Release__watchOS-Simulator_OS27.0_Apple-Watch-SE-3-40mm.txt`

#### tuist-fixtures

##### xcode-15.4.0
- completeness: **66%**
- list.json: OK
- showsdks.json: OK
- raw files: 120
- schemes: 40 (App, WatchApp, WatchAppExtension, App-Workspace, App, DynamicFrameworkB, App-Workspace, DynamicFrameworkA…)
- schemes with destinations.json: 40/40
- schemes with build-settings/: 3/40
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_destinations__Generate-Project.txt`
  - `02_list__examples_xcode_generated_app_with_buildable_folders.txt`

##### xcode-26.5.0
- completeness: **80%**
- list.json: OK
- showsdks.json: OK
- raw files: 120
- schemes: 42 (App, WatchApp, WatchAppExtension, App-Workspace, App, DynamicFrameworkB, App-Workspace, DynamicFrameworkA…)
- schemes with destinations.json: 42/42
- schemes with build-settings/: 42/42
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__WatchAppExtension__Debug__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_buildsettings__WatchAppExtension__Release__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_buildsettings__WatchApp__Debug__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_buildsettings__WatchApp__Release__iOS-Simulator_OS26.5_iPhone-17.txt`
  - `02_destinations__Generate-Project.txt`
  - `02_list__examples_xcode_generated_app_with_buildable_folders.txt`
  - `02_list__examples_xcode_generated_app_with_custom_scheme.txt`
  - `02_list__examples_xcode_generated_app_with_framework_and_tests.txt`
  - `02_list__examples_xcode_generated_command_line_tool_with_dynamic_library.txt`
  - `02_list__examples_xcode_generated_ios_app_with_coredata.txt`
  - `02_list__examples_xcode_generated_ios_app_with_custom_configuration.txt`
  - `02_list__examples_xcode_generated_ios_app_with_dynamic_frameworks_linking_static_frameworks.txt`
  - `02_list__examples_xcode_generated_ios_app_with_spm_dependencies.txt`
  - `02_list__examples_xcode_generated_ios_app_with_static_framework_with_xcstrings.txt`
  - `02_list__examples_xcode_generated_ios_app_with_static_frameworks.txt`
  - `02_list__examples_xcode_generated_ios_app_with_static_libraries.txt`
  - `02_list__examples_xcode_generated_ios_app_with_watchapp2.txt`

##### xcode-27.0.0
- completeness: **74%**
- list.json: OK
- showsdks.json: OK
- raw files: 120
- schemes: 52 (Generate Project, App, WatchApp, WatchAppExtension, App-Workspace, App, DynamicFrameworkB, App-Workspace…)
- schemes with destinations.json: 42/52
- schemes with build-settings/: 41/52
- builds: total=0, exit0=0, complete_artifacts=0
- errors:
  - `02_buildsettings__FrameworkA__debug__macOS.txt`
  - `02_buildsettings__FrameworkA__release__macOS.txt`
  - `02_buildsettings__WatchAppExtension__Debug__iOS-Simulator_OS27.0_iPhone-18-Pro.txt`
  - `02_buildsettings__WatchAppExtension__Release__iOS-Simulator_OS27.0_iPhone-18-Pro.txt`
  - `02_buildsettings__WatchApp__Debug__iOS-Simulator_OS27.0_iPhone-17.txt`
  - `02_buildsettings__WatchApp__Release__iOS-Simulator_OS27.0_iPhone-17.txt`
  - `02_destinations__Generate-Project.txt`

## Feature-probe audit

✅ = at least one capture under that fixture matches the probe; ❌ = none matches; – = not evaluable on this host (the probe walks the gitignored `corpus/<slug>/` clone, which is absent). A `*` marks a corpus-tree result preserved from the last corpus-present run (clones are pinned by `corpus/manifest.json`, so their content is stable). The **Where** column shows the first fixture with a hit.

### settings

| Probe | _global | _synthetic-custom-config | _synthetic-foreign-workspace | _synthetic-rich | _synthetic-spm-cli | _synthetic-staticlib | _synthetic-symroot-override | _synthetic-xcconfigs | _tuist-src | alamofire | ice-cubes | kingfisher | netnewswire | tuist-fixtures | Where |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| $(SRCROOT) resolved | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| $(BUILT_PRODUCTS_DIR) resolved | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| $(PROJECT_DIR) resolved | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| $(TARGET_NAME) resolved | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| $(PRODUCT_NAME) resolved | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| $(EFFECTIVE_PLATFORM_NAME) resolved | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-foreign-workspace: present in 1 target × config × dest combos |
| Non-Debug/Release configuration | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | _synthetic-custom-config: CONFIGURATION='Profile' (1 hits) |
| ARCHS_STANDARD resolved | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| VALID_ARCHS present | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| EXCLUDED_ARCHS set | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: EXCLUDED_ARCHS='arm64e' (2 hits) |
| x86_64 architecture present | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: ARCHS='arm64 x86_64' (3 hits) |
| arm64e architecture present | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: ARCHS='arm64e' (2 hits) |
| Static library MACH_O_TYPE | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: MACH_O_TYPE='staticlib' (224 hits) |
| Dynamic library MACH_O_TYPE | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ✅ | alamofire: MACH_O_TYPE='mh_dylib' (4 hits) |
| Executable MACH_O_TYPE | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: MACH_O_TYPE='mh_execute' (3 hits) |
| Bundle MACH_O_TYPE | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: MACH_O_TYPE='mh_bundle' (112 hits) |
| LD_RUNPATH_SEARCH_PATHS | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-foreign-workspace: present in 1 target × config × dest combos |
| OTHER_LDFLAGS | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | _synthetic-xcconfigs: present in 1 target × config × dest combos |
| Mergeable libraries | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: MERGEABLE_LIBRARY='YES' (2 hits) |
| Link-time optimization | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: LLVM_LTO='YES' (4 hits) |
| Mac Catalyst supported | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | alamofire: SUPPORTS_MACCATALYST='YES' (2 hits) |
| Built as Mac Catalyst | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | alamofire: IS_MACCATALYST='YES' (2 hits) |
| Designed for iPad on Mac | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | alamofire: SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD='YES' (4 hits) |
| iphonesimulator SDK | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-foreign-workspace: SDKROOT='/Applications/Xcode-26.5.0.app/Contents/Developer/Platforms/iPhoneSimulator.plat' (1 hits) |
| macosx SDK | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: SDKROOT='/Applications/Xcode-15.4.0.app/Contents/Developer/Platforms/MacOSX.platform/Deve' (3 hits) |
| watchsimulator SDK | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ✅ | alamofire: SDKROOT='/Applications/Xcode-26.5.0.app/Contents/Developer/Platforms/WatchSimulator.platf' (4 hits) |
| appletvsimulator SDK | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ | alamofire: SDKROOT='/Applications/Xcode-26.5.0.app/Contents/Developer/Platforms/AppleTVSimulator.pla' (2 hits) |
| xrsimulator SDK | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | alamofire: SDKROOT='/Applications/Xcode-26.5.0.app/Contents/Developer/Platforms/XRSimulator.platform' (2 hits) |
| driverkit SDK | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| Asset catalog with AppIcon | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | alamofire: present in 10 target × config × dest combos |
| Info.plist explicitly listed | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | alamofire: INFOPLIST_FILE='Source/Info.plist' (4 hits) |
| Info.plist generated from build settings | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | _synthetic-foreign-workspace: GENERATE_INFOPLIST_FILE='YES' (1 hits) |
| CODE_SIGN_ENTITLEMENTS set | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ice-cubes: CODE_SIGN_ENTITLEMENTS='IceCubesApp/App/IceCubesApp-release.entitlements' (54 hits) |
| SWIFT_VERSION declared | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-custom-config: present in 3 target × config × dest combos |
| Library evolution (BUILD_LIBRARY_FOR_DISTRIBUTION=YES) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: BUILD_LIBRARY_FOR_DISTRIBUTION='YES' (4 hits) |
| SWIFT_STRICT_CONCURRENCY explicit | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | alamofire: SWIFT_STRICT_CONCURRENCY='complete' (4 hits) |
| Swift upcoming feature: strict concurrency | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: SWIFT_UPCOMING_FEATURE_STRICT_CONCURRENCY='YES' (2 hits) |
| OTHER_SWIFT_FLAGS non-empty | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | _synthetic-xcconfigs: OTHER_SWIFT_FLAGS=' -DMY_FLAG' (1 hits) |
| Obj-C bridging header | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | netnewswire: SWIFT_OBJC_BRIDGING_HEADER='Mac/NetNewsWire-Bridging-Header.h' (12 hits) |
| DEFINES_MODULE=YES | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | kingfisher: DEFINES_MODULE='YES' (2 hits) |
| ObjC ARC enabled | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-foreign-workspace: CLANG_ENABLE_OBJC_ARC='YES' (1 hits) |
| ObjC weak refs enabled | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | _synthetic-foreign-workspace: CLANG_ENABLE_OBJC_WEAK='YES' (1 hits) |
| Product: application | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | _synthetic-foreign-workspace: PRODUCT_TYPE='com.apple.product-type.application' (1 hits) |
| Product: framework | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ✅ | alamofire: PRODUCT_TYPE='com.apple.product-type.framework' (4 hits) |
| Product: static library | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: PRODUCT_TYPE='com.apple.product-type.library.static' (16 hits) |
| Product: dynamic library | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: PRODUCT_TYPE='com.apple.product-type.library.dynamic' (4 hits) |
| Product: resource bundle | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: PRODUCT_TYPE='com.apple.product-type.bundle' (60 hits) |
| Product: command-line tool | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | _synthetic-custom-config: PRODUCT_TYPE='com.apple.product-type.tool' (3 hits) |
| Product: XPC service | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| Product: unit-test bundle | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: PRODUCT_TYPE='com.apple.product-type.bundle.unit-test' (48 hits) |
| Product: UI-test bundle | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: PRODUCT_TYPE='com.apple.product-type.bundle.ui-testing' (4 hits) |
| Product: app-extension | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ice-cubes: PRODUCT_TYPE='com.apple.product-type.app-extension' (24 hits) |
| Product: DriverKit driver extension | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| OTHER_LDFLAGS contains quoted whitespace | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | alamofire: OTHER_LDFLAGS='-framework "My Framework" -Wl,-segalign,0x4000' (2 hits) |

### xcconfig

| Probe | _global | _synthetic-custom-config | _synthetic-foreign-workspace | _synthetic-rich | _synthetic-spm-cli | _synthetic-staticlib | _synthetic-symroot-override | _synthetic-xcconfigs | _tuist-src | alamofire | ice-cubes | kingfisher | netnewswire | tuist-fixtures | Where |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| $(inherited) used in xcconfig | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | _synthetic-xcconfigs: xcconfig matched: xcconfigs/inherited.xcconfig (+0) |
| Recursive substitution in xcconfig (>=2 refs in one value) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | netnewswire: xcconfig matched: raw/xcconfig/NetNewsWire_safariextension_target.xcconfig (+8) |
| Modifier syntax ${VAR:lower}/${VAR:default=...} in xcconfig | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | _synthetic-xcconfigs: xcconfig matched: xcconfigs/modifier-syntax.xcconfig (+0) |
| Multi-line continuation in xcconfig | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | _synthetic-xcconfigs: xcconfig matched: xcconfigs/multi-line-continuation.xcconfig (+0) |
| Conditional [sdk=...] in xcconfig | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | _synthetic-xcconfigs: xcconfig matched: xcconfigs/conditional-sdk.xcconfig (+0) |
| Conditional [arch=...] in xcconfig | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | _synthetic-xcconfigs: xcconfig matched: xcconfigs/conditional-arch.xcconfig (+0) |
| Conditional [config=...] in xcconfig | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | _synthetic-custom-config: xcconfig matched: project/Shared.xcconfig (+0) |
| xcconfig #include directive | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | _synthetic-xcconfigs: xcconfig matched: xcconfigs/include-directive.xcconfig (+0) |

### pbxproj

| Probe | _global | _synthetic-custom-config | _synthetic-foreign-workspace | _synthetic-rich | _synthetic-spm-cli | _synthetic-staticlib | _synthetic-symroot-override | _synthetic-xcconfigs | _tuist-src | alamofire | ice-cubes | kingfisher | netnewswire | tuist-fixtures | Where |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Conditional [sdk=...] in pbxproj | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | alamofire: pbxproj matched: raw/Example/iOS Example.xcodeproj/project.pbxproj (+1) |
| Conditional [arch=...] in pbxproj | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| .xcconfig referenced from pbxproj | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | _synthetic-custom-config: pbxproj matched: project/Scratch.xcodeproj/project.pbxproj (+0) |
| Remote SwiftPM dependency (pbxproj) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ice-cubes: pbxproj matched: raw/IceCubesApp.xcodeproj/project.pbxproj (+0) |
| SwiftPM product dependency (pbxproj) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ice-cubes: pbxproj matched: raw/IceCubesApp.xcodeproj/project.pbxproj (+0) |
| Run Script build phase | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | netnewswire: pbxproj matched: raw/NetNewsWire.xcodeproj/project.pbxproj (+0) |
| Headers build phase (public/private) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | kingfisher: pbxproj matched: raw/Kingfisher.xcodeproj/project.pbxproj (+0) |
| Copy Files build phase | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | _tuist-src: pbxproj matched: raw/CommandLineTool.xcodeproj/project.pbxproj (+0) |
| Embed Frameworks copy phase | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | _tuist-src: pbxproj matched: raw/CommandLineTool.xcodeproj/project.pbxproj (+0) |
| PBXTargetDependency | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | _tuist-src: pbxproj matched: raw/CommandLineTool.xcodeproj/project.pbxproj (+0) |
| Cross-project container reference | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | _tuist-src: pbxproj matched: raw/CommandLineTool.xcodeproj/project.pbxproj (+0) |

### scheme

| Probe | _global | _synthetic-custom-config | _synthetic-foreign-workspace | _synthetic-rich | _synthetic-spm-cli | _synthetic-staticlib | _synthetic-symroot-override | _synthetic-xcconfigs | _tuist-src | alamofire | ice-cubes | kingfisher | netnewswire | tuist-fixtures | Where |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Scheme pre-action defined | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | netnewswire: xcscheme matched: raw/NetNewsWire.xcodeproj/xcshareddata/xcschemes/NetNewsWire-iOS.xcscheme (+1) |
| Scheme post-action defined | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| Scheme env vars | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | alamofire: xcscheme matched: raw/Example/iOS Example.xcodeproj/xcshareddata/xcschemes/iOS Example.xcscheme (+0) |
| Scheme launch arguments | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: xcscheme matched: raw/examples_xcode_generated_ios_app_with_watchapp2/App.xcworkspace/xcshareddata/xcschemes/Generate Project.xcscheme (+9) |
| Scheme test plan reference | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ❌ | alamofire: xcscheme matched: raw/Alamofire.xcodeproj/xcshareddata/xcschemes/Alamofire macOS.xcscheme (+4) |

### files

| Probe | _global | _synthetic-custom-config | _synthetic-foreign-workspace | _synthetic-rich | _synthetic-spm-cli | _synthetic-staticlib | _synthetic-symroot-override | _synthetic-xcconfigs | _tuist-src | alamofire | ice-cubes | kingfisher | netnewswire | tuist-fixtures | Where |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| .xcstrings file present | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ice-cubes: corpus/ has 1 .xcstrings files (e.g. Localizable.xcstrings) |
| Legacy .strings file present | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ice-cubes: corpus/ has 12 .strings files (e.g. InfoPlist.strings) |
| .storyboard file present | – | – | – | ❌ | – | ❌ | – | – | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ | alamofire: corpus/ has 3 .storyboard files (e.g. LaunchScreen.storyboard) |
| .xib file present | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | kingfisher: corpus/ has 1 .xib files (e.g. Cell.xib) |
| Asset catalog Contents.json | – | – | – | ❌ | – | ❌ | – | – | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | alamofire: corpus/ has 16 Contents.json files |
| Core Data .xcdatamodeld bundle | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: corpus/ has 5 .xcdatamodeld bundle dirs (e.g. Users.xcdatamodeld) |
| Core ML .mlmodel | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| Metal .metal shader | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | tuist-fixtures: corpus/ has 2 .metal files (e.g. Metal.metal) |
| PrivacyInfo.xcprivacy | – | – | – | ❌ | – | ❌ | – | – | ❌ | ✅ | ❌ | ✅ | ✅ | ❌ | alamofire: corpus/ has 1 .xcprivacy files (e.g. PrivacyInfo.xcprivacy) |
| .entitlements file present | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | ice-cubes: raw/ has 6 .entitlements files (e.g. IceCubesApp-release.entitlements) |
| App Groups entitlement | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ice-cubes: .entitlements matched: IceCubesApp-release.entitlements |
| iCloud entitlement | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ice-cubes: .entitlements matched: IceCubesApp-release.entitlements |
| Push notifications entitlement | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ice-cubes: .entitlements matched: IceCubesApp-release.entitlements |
| WiFi info entitlement | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| Package.swift in corpus | – | – | – | ❌ | – | ❌ | – | – | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | alamofire: corpus/ has 1 Package.swift files |
| Package.resolved in corpus | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ice-cubes: corpus/ has 1 Package.resolved files |
| Obj-C .m source present | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | kingfisher: corpus/ has 25 .m files (e.g. LSNocilla.m) |
| Obj-C++ .mm source present | – | – | – | ✅ | – | ✅ | – | – | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | _synthetic-rich: corpus/ has 1 .mm files (e.g. util.mm) |
| Pre-compiled header .pch | – | – | – | ❌ | – | ❌ | – | – | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | — |
| .xctestplan file present | – | – | – | ❌ | – | ❌ | – | – | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | alamofire: corpus/ has 5 .xctestplan files (e.g. watchOS.xctestplan) |
