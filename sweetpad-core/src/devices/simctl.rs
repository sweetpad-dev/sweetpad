//! `xcrun simctl` output: the simulator listing and the host processes a
//! simulator app runs as. The CLI spawns simctl and `ps`; the extension does
//! the same and hands the output here through the addon.

use std::cmp::Ordering;

use serde_json::Value;
use sweetpad_lib::destination::{DestinationSpec, PLATFORMS, Platform};

/// A simulator, with its runtime parsed into a friendly OS + version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Simulator {
    pub udid: String,
    pub name: String,
    /// `Booted` / `Shutdown` (as reported by simctl).
    pub state: String,
    pub available: bool,
    /// e.g. `iOS`, `watchOS`, `tvOS`, `xrOS`.
    pub os: String,
    /// e.g. `17.0`.
    pub os_version: String,
    /// The runtime identifier, e.g. `com.apple.CoreSimulator.SimRuntime.iOS-17-0`.
    pub runtime: String,
    /// The device type identifier, e.g.
    /// `com.apple.CoreSimulator.SimDeviceType.iPhone-17`. Empty when simctl
    /// leaves it out.
    pub device_type: String,
}

impl Simulator {
    #[must_use]
    pub fn is_booted(&self) -> bool {
        self.state.eq_ignore_ascii_case("Booted")
    }

    /// `"iPhone 15 (17.0)"`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.os_version)
    }

    /// The simulator platform this OS runs, when the platform table knows it.
    #[must_use]
    pub fn platform(&self) -> Option<&'static Platform> {
        Platform::simulator_for_os(&self.os)
    }

    /// The `xcodebuild -destination` platform label, e.g. `iOS Simulator`.
    /// An OS the platform table doesn't know keeps its own name (`fooOS
    /// Simulator`), the likeliest spelling for a platform newer than the
    /// table.
    #[must_use]
    pub fn platform_label(&self) -> String {
        self.platform()
            .map_or_else(|| format!("{} Simulator", self.os), |p| p.label.to_string())
    }

    /// The `xcodebuild -destination` specifier targeting this simulator,
    /// e.g. `platform=iOS Simulator,id=<udid>`.
    #[must_use]
    pub fn destination(&self) -> String {
        format!("platform={},id={}", self.platform_label(), self.udid)
    }

    /// Destination kind for the remembered recents/usage records, e.g.
    /// `iOSSimulator`. Pairs the OS with the simulator role.
    #[must_use]
    pub fn kind(&self) -> String {
        format!("{}Simulator", self.os)
    }
}

/// Parse `simctl list --json devices` output into available simulators in
/// picker order: platform first (iOS before the rest), then newest OS version,
/// then device family (iPhone before iPad) and a numeric-aware name sort.
/// Unavailable devices are dropped (they can't be booted or targeted).
pub fn parse_list(raw: &str) -> Result<Vec<Simulator>, String> {
    // simctl's stdout can carry a warning ahead of the JSON (a CoreSimulator
    // notice); the listing is the first object in it.
    let json = raw.find('{').map_or(raw, |start| &raw[start..]);
    let parsed: Value = serde_json::Deserializer::from_str(json)
        .into_iter::<Value>()
        .next()
        .unwrap_or_else(|| serde_json::from_str::<Value>(""))
        .map_err(|e| format!("parsing simctl output: {e}"))?;
    let devices = parsed
        .get("devices")
        .and_then(Value::as_object)
        .ok_or("parsing simctl output: it has no 'devices' map")?;
    let text = |d: &Value, key: &str| d[key].as_str().unwrap_or_default().to_string();
    let mut sims = Vec::new();
    for (runtime, list) in devices {
        let (os, os_version) = parse_runtime(runtime);
        for d in list.as_array().into_iter().flatten() {
            let udid = text(d, "udid");
            if udid.is_empty() || d["isAvailable"].as_bool() != Some(true) {
                continue;
            }
            sims.push(Simulator {
                udid,
                name: text(d, "name"),
                state: text(d, "state"),
                available: true,
                os: os.clone(),
                os_version: os_version.clone(),
                runtime: runtime.clone(),
                device_type: text(d, "deviceTypeIdentifier"),
            });
        }
    }
    sims.sort_by(cmp_for_picker);
    Ok(sims)
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-17-0` → (`iOS`, `17.0`). A runtime
/// without a version keeps its whole tail as the OS.
#[must_use]
pub fn parse_runtime(runtime: &str) -> (String, String) {
    let tail = runtime.rsplit('.').next().unwrap_or(runtime); // iOS-17-0
    match tail.split_once('-') {
        Some((os, version)) => (os.to_string(), version.replace('-', ".")),
        None => (tail.to_string(), String::new()),
    }
}

/// Order simulators for pickers and listings: platform priority, then newest OS
/// version, then device family, then a numeric-aware name sort. Each tier is
/// explicit so the order is intentional rather than a byte-compare side effect
/// (which is what made 17.0 sort before 9.0 and "iPad" before "iPhone").
fn cmp_for_picker(a: &Simulator, b: &Simulator) -> Ordering {
    platform_rank(&a.os)
        .cmp(&platform_rank(&b.os))
        .then_with(|| version_key(&b.os_version).cmp(&version_key(&a.os_version))) // newest first
        .then_with(|| device_rank(&a.name).cmp(&device_rank(&b.name)))
        .then_with(|| natural_cmp(&a.name, &b.name))
}

/// Platform display order: the platform table's (iOS, tvOS, watchOS,
/// visionOS); anything unrecognized sorts last, so a future platform lands in a
/// defined place rather than wherever its name's bytes happen to fall.
fn platform_rank(os: &str) -> usize {
    Platform::simulator_for_os(os)
        .and_then(|p| PLATFORMS.iter().position(|q| q == p))
        .unwrap_or(PLATFORMS.len())
}

/// Device-family order within a platform: iPhone before iPad (the common pick),
/// then everything else. Platforms with a single family (Apple TV/Watch/Vision)
/// all land in the last bucket and fall through to the name sort.
fn device_rank(name: &str) -> u8 {
    if name.starts_with("iPhone") {
        0
    } else if name.starts_with("iPad") {
        1
    } else {
        2
    }
}

/// Parse a dotted version ("26.5") into numeric components so it orders
/// numerically: 9.0 before 17.0, where a byte compare puts "17.0" first.
/// Missing or garbled components count as 0.
#[must_use]
pub fn version_key(version: &str) -> Vec<u32> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Compare names so embedded numbers order numerically: "iPhone 9" before
/// "iPhone 15", which a plain byte compare reverses. Digit runs compare as
/// numbers; everything else compares byte-wise.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                match take_number(&mut a).cmp(&take_number(&mut b)) {
                    Ordering::Equal => {}
                    ord => return ord,
                }
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                match x.cmp(&y) {
                    Ordering::Equal => {}
                    ord => return ord,
                }
            }
        }
    }
}

/// Consume a leading run of digits as a number (saturating, so a pathologically
/// long run can't overflow).
fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> u64 {
    let mut n: u64 = 0;
    while let Some(d) = it.peek().and_then(|c| c.to_digit(10)) {
        n = n.saturating_mul(10).saturating_add(u64::from(d));
        it.next();
    }
    n
}

/// Find a simulator by UDID (case-insensitive) or exact name. When several
/// share a name, the booted one wins, else the first.
#[must_use]
pub fn find<'a>(sims: &'a [Simulator], query: &str) -> Option<&'a Simulator> {
    if let Some(s) = sims.iter().find(|s| s.udid.eq_ignore_ascii_case(query)) {
        return Some(s);
    }
    let mut by_name: Vec<&Simulator> = sims.iter().filter(|s| s.name == query).collect();
    by_name.sort_by_key(|s| !s.is_booted());
    by_name.first().copied()
}

/// The simulator a `-destination` names by `name=`, matched the way xcodebuild
/// matches it: the name exactly, on the destination's platform when it gives
/// one, at its `OS=` exactly (`27` is not `27.0`), or at the newest OS for
/// `OS=latest`. A booted simulator wins a tie. `None` when the destination has
/// no `name=` or nothing matches. Xcode keeps a default set of devices for
/// every runtime, so `iPhone 17` can name one simulator per installed iOS.
#[must_use]
pub fn find_named<'a>(sims: &'a [Simulator], spec: &DestinationSpec) -> Option<&'a Simulator> {
    let name = spec.name.as_deref()?;
    let mut matches: Vec<&Simulator> = sims
        .iter()
        .filter(|s| s.name == name && spec.platform.is_none_or(|p| s.platform() == Some(p)))
        .collect();
    match spec.os.as_deref() {
        Some(os) if os.eq_ignore_ascii_case("latest") => {
            let newest = matches.iter().map(|s| version_key(&s.os_version)).max();
            matches.retain(|s| Some(version_key(&s.os_version)) == newest);
        }
        Some(os) => matches.retain(|s| s.os_version == os),
        None => {}
    }
    matches.sort_by_key(|s| !s.is_booted());
    matches.first().copied()
}

/// The pids in `ps -o pid=,comm=` output of `executable` running out of an
/// `.app` named `app_dir` on the simulator `udid`. A simulator app is a host
/// process under the device's data directory, so the host's `ps` sees it
/// without asking the simulator. The command path runs through
/// `CoreSimulator/Devices/<udid>/` (simctl prints the udid in upper case, a
/// typed destination may not) and ends in `<app_dir>/<executable>`.
#[must_use]
pub fn parse_app_pids(ps: &str, udid: &str, app_dir: &str, executable: &str) -> Vec<u32> {
    const DEVICES: &str = "/CoreSimulator/Devices/";
    let tail = format!("/{app_dir}/{executable}");
    ps.lines()
        .filter_map(|line| {
            let (pid, comm) = line.trim_start().split_once(' ')?;
            let (_, under) = comm.split_once(DEVICES)?;
            let device = under.split('/').next()?;
            (device.eq_ignore_ascii_case(udid) && comm.ends_with(&tail))
                .then(|| pid.parse().ok())?
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
          {"udid":"AAAA","name":"iPhone 15","state":"Booted","isAvailable":true},
          {"udid":"BBBB","name":"iPhone 14","state":"Shutdown","isAvailable":true},
          {"udid":"DEAD","name":"Old","state":"Shutdown","isAvailable":false}
        ],
        "com.apple.CoreSimulator.SimRuntime.watchOS-10-0": [
          {"udid":"CCCC","name":"Apple Watch","state":"Shutdown","isAvailable":true}
        ]
      }
    }"#;

    #[test]
    fn parses_and_filters_unavailable() {
        let sims = parse_list(SAMPLE).unwrap();
        // The unavailable "Old" device is dropped.
        assert_eq!(sims.len(), 3);
        assert!(sims.iter().all(|s| s.udid != "DEAD"));
    }

    #[test]
    fn sorts_ios_before_watchos_then_by_name() {
        let sims = parse_list(SAMPLE).unwrap();
        let order: Vec<&str> = sims.iter().map(|s| s.name.as_str()).collect();
        // iOS before watchOS; within iOS, name order.
        assert_eq!(order, vec!["iPhone 14", "iPhone 15", "Apple Watch"]);
    }

    // Several runtimes and families, to exercise every ordering tier at once.
    const MIXED: &str = r#"{
      "devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
          {"udid":"A","name":"iPhone 15","state":"Shutdown","isAvailable":true},
          {"udid":"B","name":"iPhone 9","state":"Shutdown","isAvailable":true},
          {"udid":"C","name":"iPad Air","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
          {"udid":"D","name":"iPhone 14","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.tvOS-26-0": [
          {"udid":"E","name":"Apple TV","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [
          {"udid":"F","name":"Apple Watch","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.xrOS-2-0": [
          {"udid":"G","name":"Apple Vision Pro","state":"Shutdown","isAvailable":true}
        ]
      }
    }"#;

    #[test]
    fn picker_order_is_platform_then_newest_then_family_then_natural() {
        let sims = parse_list(MIXED).unwrap();
        let order: Vec<&str> = sims.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            order,
            vec![
                // iOS first; newest (26.5) before 17.0; iPhone before iPad;
                // "iPhone 9" before "iPhone 15" (numeric, not byte order).
                "iPhone 9",
                "iPhone 15",
                "iPad Air",
                "iPhone 14",
                // then the remaining platforms in priority order.
                "Apple TV",
                "Apple Watch",
                "Apple Vision Pro",
            ]
        );
    }

    /// `simctl list -j` on Xcode 27, trimmed to one device per runtime and
    /// the fields sweetpad reads.
    const XCODE_27: &str = r#"{
      "devicetypes": [],
      "runtimes": [],
      "devices": {
        "com.apple.CoreSimulator.SimRuntime.watchOS-27-0": [
          {"lastBootedAt": "2026-09-20T10:00:00Z", "dataPath": "/x", "dataPathSize": 1, "logPath": "/y", "udid": "ABD4EBF9-910F-4A29-89D7-9B40DB2D7D18", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-12-46mm", "state": "Shutdown", "name": "Apple Watch Series 12 (46mm)"}
        ],
        "com.apple.CoreSimulator.SimRuntime.tvOS-27-0": [
          {"udid": "2CD2A3F5-8763-46B5-B7FC-F04117966B44", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K", "state": "Shutdown", "name": "Apple TV 4K (3rd generation)"}
        ],
        "com.apple.CoreSimulator.SimRuntime.iOS-27-0": [
          {"udid": "F13C004A-0824-4870-B4F2-29AAEE36636E", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-17", "state": "Booted", "name": "iPhone 17"}
        ],
        "com.apple.CoreSimulator.SimRuntime.xrOS-27-0": [
          {"udid": "1D09DF49-C00E-42A6-90D7-6D1997FFEA6D", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro-4K", "state": "Shutdown", "name": "Apple Vision Pro"}
        ]
      }
    }"#;

    #[test]
    fn every_xcode_27_runtime_gets_its_own_destination() {
        let sims = parse_list(XCODE_27).unwrap();
        let destinations: Vec<String> = sims.iter().map(Simulator::destination).collect();
        assert_eq!(
            destinations,
            [
                "platform=iOS Simulator,id=F13C004A-0824-4870-B4F2-29AAEE36636E",
                "platform=tvOS Simulator,id=2CD2A3F5-8763-46B5-B7FC-F04117966B44",
                "platform=watchOS Simulator,id=ABD4EBF9-910F-4A29-89D7-9B40DB2D7D18",
                "platform=visionOS Simulator,id=1D09DF49-C00E-42A6-90D7-6D1997FFEA6D",
            ]
        );
        let vision = &sims[3];
        assert_eq!(
            (vision.os.as_str(), vision.os_version.as_str()),
            ("xrOS", "27.0")
        );
        assert_eq!(vision.platform().unwrap().sdk, "xrsimulator");
        assert_eq!(
            vision.device_type,
            "com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro-4K"
        );
        assert_eq!(
            vision.runtime,
            "com.apple.CoreSimulator.SimRuntime.xrOS-27-0"
        );
        assert!(sims[0].is_booted());
    }

    #[test]
    fn an_unknown_os_keeps_its_own_platform_name() {
        let sim = Simulator {
            udid: "U".into(),
            os: "fooOS".into(),
            ..Simulator::default()
        };
        assert_eq!(sim.destination(), "platform=fooOS Simulator,id=U");
    }

    #[test]
    fn a_listing_without_devices_is_an_error() {
        assert!(parse_list("{}").is_err());
        assert!(parse_list("not json").is_err());
        assert!(parse_list("").is_err());
    }

    #[test]
    fn a_notice_around_the_json_is_skipped() {
        let raw = format!("CoreSimulator notice: something\n{SAMPLE}\ntrailing line\n");
        assert_eq!(parse_list(&raw).unwrap().len(), 3);
    }

    #[test]
    fn natural_cmp_orders_numbers_numerically() {
        assert_eq!(natural_cmp("iPhone 9", "iPhone 15"), Ordering::Less);
        assert_eq!(natural_cmp("iPhone 15", "iPhone 15"), Ordering::Equal);
        assert_eq!(natural_cmp("iPhone 15 Pro", "iPhone 15"), Ordering::Greater);
        // A byte compare would put "iPhone 15" before "iPhone 9"; this must not.
        assert_eq!("iPhone 15".cmp("iPhone 9"), Ordering::Less);
        assert_eq!(natural_cmp("iPhone 15", "iPhone 9"), Ordering::Greater);
    }

    #[test]
    fn version_key_compares_numerically() {
        assert!(version_key("9.0") < version_key("17.0"));
        assert!(version_key("26.5") > version_key("26.4"));
        assert_eq!(version_key("26.5"), vec![26, 5]);
    }

    #[test]
    fn ranks_put_ios_and_iphone_first() {
        assert!(platform_rank("iOS") < platform_rank("tvOS"));
        assert!(platform_rank("tvOS") < platform_rank("watchOS"));
        assert!(platform_rank("watchOS") < platform_rank("xrOS"));
        assert!(platform_rank("xrOS") < platform_rank("unknownOS"));
        assert!(device_rank("iPhone 15") < device_rank("iPad Air"));
        assert!(device_rank("iPad Air") < device_rank("Apple TV"));
    }

    #[test]
    fn parses_runtime_into_os_and_version() {
        assert_eq!(
            parse_runtime("com.apple.CoreSimulator.SimRuntime.iOS-17-0"),
            ("iOS".to_string(), "17.0".to_string())
        );
        assert_eq!(
            parse_runtime("com.apple.CoreSimulator.SimRuntime.watchOS-10-2"),
            ("watchOS".to_string(), "10.2".to_string())
        );
        assert_eq!(
            parse_runtime("com.apple.CoreSimulator.SimRuntime.iOS-26-0-1"),
            ("iOS".to_string(), "26.0.1".to_string())
        );
    }

    #[test]
    fn destination_specifier_maps_platform() {
        let sims = parse_list(SAMPLE).unwrap();
        let watch = sims.iter().find(|s| s.os == "watchOS").unwrap();
        assert_eq!(
            watch.destination(),
            format!("platform=watchOS Simulator,id={}", watch.udid)
        );
        let iphone = sims.iter().find(|s| s.name == "iPhone 15").unwrap();
        assert_eq!(iphone.destination(), "platform=iOS Simulator,id=AAAA");
    }

    #[test]
    fn find_matches_udid_case_insensitively() {
        let sims = parse_list(SAMPLE).unwrap();
        assert_eq!(find(&sims, "aaaa").unwrap().name, "iPhone 15");
    }

    #[test]
    fn find_by_name_prefers_booted() {
        let sim = |udid: &str, state: &str| Simulator {
            udid: udid.into(),
            name: "Dup".into(),
            state: state.into(),
            available: true,
            os: "iOS".into(),
            os_version: "17.0".into(),
            ..Simulator::default()
        };
        let sims = vec![sim("1", "Shutdown"), sim("2", "Booted")];
        assert_eq!(find(&sims, "Dup").unwrap().udid, "2");
    }

    /// Two runtimes' default sets both hold an `iPhone 17`; the destination's
    /// `OS=` and platform decide which one it names, as they do for
    /// xcodebuild.
    #[test]
    fn a_named_destination_honors_its_platform_and_os() {
        let sim = |udid: &str, name: &str, os: &str, version: &str, state: &str| Simulator {
            udid: udid.into(),
            name: name.into(),
            state: state.into(),
            available: true,
            os: os.into(),
            os_version: version.into(),
            ..Simulator::default()
        };
        let sims = vec![
            sim("NEW", "iPhone 17", "iOS", "27.0", "Booted"),
            sim("OLD", "iPhone 17", "iOS", "26.5", "Shutdown"),
            sim("WATCH", "Twin", "watchOS", "27.0", "Booted"),
            sim("PHONE", "Twin", "iOS", "27.0", "Shutdown"),
        ];
        let named = |destination: &str| {
            find_named(&sims, &DestinationSpec::parse(destination)).map(|s| s.udid.as_str())
        };
        assert_eq!(
            named("platform=iOS Simulator,name=iPhone 17,OS=26.5"),
            Some("OLD")
        );
        assert_eq!(
            named("platform=iOS Simulator,name=iPhone 17,OS=27.0"),
            Some("NEW")
        );
        assert_eq!(
            named("platform=iOS Simulator,name=iPhone 17,OS=latest"),
            Some("NEW")
        );
        // `OS=27` names no runtime, for xcodebuild or here.
        assert_eq!(named("platform=iOS Simulator,name=iPhone 17,OS=27"), None);
        // Without an OS, the booted one wins.
        assert_eq!(named("platform=iOS Simulator,name=iPhone 17"), Some("NEW"));
        // The platform keeps a booted watch from standing in for an iPhone.
        assert_eq!(named("platform=iOS Simulator,name=Twin"), Some("PHONE"));
        assert_eq!(named("platform=watchOS Simulator,name=Twin"), Some("WATCH"));
        assert_eq!(named("platform=iOS Simulator,id=NEW"), None);
    }

    #[test]
    fn label_includes_version() {
        let s = Simulator {
            udid: "x".into(),
            name: "iPhone 15".into(),
            state: "Booted".into(),
            available: true,
            os: "iOS".into(),
            os_version: "17.0".into(),
            ..Simulator::default()
        };
        assert_eq!(s.label(), "iPhone 15 (17.0)");
        assert!(s.is_booted());
    }

    #[test]
    fn app_pids_match_the_app_on_that_simulator_only() {
        let ps = "\
  101 /Users/me/Library/Developer/CoreSimulator/Devices/5E88C8CE-7810-45BA-AAC5-5A780BF13348/data/Containers/Bundle/Application/7370/My App.app/My App
  102 /Users/me/Library/Developer/CoreSimulator/Devices/F13C004A-0824-4870-B4F2-29AAEE36636E/data/Containers/Bundle/Application/1111/My App.app/My App
  103 /Users/me/Library/Developer/Xcode/DerivedData/X/Build/Products/Debug-iphonesimulator/My App.app/My App
  104 /Users/me/Library/Developer/CoreSimulator/Devices/5E88C8CE-7810-45BA-AAC5-5A780BF13348/data/Containers/Bundle/Application/7371/Other.app/Other
  105 /usr/libexec/launchd_sim";
        let udid = "5e88c8ce-7810-45ba-aac5-5a780bf13348";
        assert_eq!(parse_app_pids(ps, udid, "My App.app", "My App"), vec![101]);
        assert!(parse_app_pids(ps, udid, "Missing.app", "Missing").is_empty());
    }
}
