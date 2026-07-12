//! Native macOS app window capture (CLI_DESIGN §9h): find the running app's
//! windows via `CGWindowListCopyWindowInfo`, capture one with
//! `screencapture -l`, and preflight the Screen Recording permission that
//! capture (but not enumeration) requires.
//!
//! The FFI block is deliberately tiny — four CoreGraphics calls and the
//! CoreFoundation accessors to walk the returned array of dictionaries —
//! so no binding crate is pulled in. Everything above it (`ps` parsing,
//! window filtering and picking) is pure and unit-tested.

use std::ffi::c_void;
use std::path::Path;

use crate::cli::CliError;
use crate::cli::process;

/// One on-screen window, as reported by the window server. The list order is
/// front-to-back, so index 0 of a pid-filtered list is that app's frontmost
/// window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    /// `kCGWindowNumber` — the id `screencapture -l` takes.
    pub number: u32,
    pub pid: i32,
    /// `kCGWindowLayer`; 0 is the normal document/app layer (menu bar,
    /// status items, and other chrome live on higher layers).
    pub layer: i32,
    /// `kCGWindowOwnerName` — the app's display name.
    pub owner: String,
    /// `kCGWindowName` — the title; populated only once Screen Recording is
    /// granted, so it's decoration for error listings, never matched on.
    pub title: Option<String>,
    pub width: f64,
    pub height: f64,
    /// `kCGWindowAlpha`; 0 is an invisible window that would capture as
    /// nothing.
    pub alpha: f64,
}

impl WindowInfo {
    /// Whether this window is worth capturing: the normal app layer and
    /// actually visible.
    fn capturable(&self) -> bool {
        self.layer == 0 && self.alpha > 0.0 && self.width >= 2.0 && self.height >= 2.0
    }

    /// `1512×982 "Title"` — for listings in errors.
    fn describe(&self) -> String {
        let size = format!("{}×{}", self.width.round(), self.height.round());
        match &self.title {
            Some(t) if !t.is_empty() => format!("{size} {t:?}"),
            _ => size,
        }
    }
}

type CFTypeRef = *const c_void;
type CFIndex = isize;

const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const KCF_NUMBER_SINT64_TYPE: CFIndex = 4;
const KCF_NUMBER_FLOAT64_TYPE: CFIndex = 6;
const KCG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
const KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const KCG_NULL_WINDOW_ID: u32 = 0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const i8, encoding: u32) -> CFTypeRef;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut i8, size: CFIndex, encoding: u32) -> u8;
    fn CFNumberGetValue(number: CFTypeRef, number_type: CFIndex, out: *mut c_void) -> u8;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFTypeRef;
    fn CGPreflightScreenCaptureAccess() -> u8;
    fn CGRequestScreenCaptureAccess() -> u8;
}

/// Whether this process (attributed to the hosting terminal app) already
/// holds the Screen Recording permission.
pub fn has_screen_capture_access() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() != 0 }
}

/// Trigger the one-time OS permission prompt (registers the hosting app in
/// System Settings → Privacy & Security → Screen Recording). Returns
/// immediately; the grant lands on a future run.
pub fn request_screen_capture_access() {
    unsafe {
        CGRequestScreenCaptureAccess();
    }
}

/// A dictionary key held only for a lookup, released on drop.
struct CFKey(CFTypeRef);

impl CFKey {
    fn new(name: &str) -> Self {
        let c = std::ffi::CString::new(name).expect("static key");
        CFKey(unsafe {
            CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8)
        })
    }
}

impl Drop for CFKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

/// Read a CFNumber dictionary value as `f64` (converting from whatever width
/// the window server stored).
unsafe fn dict_f64(dict: CFTypeRef, key: &CFKey) -> Option<f64> {
    let value = unsafe { CFDictionaryGetValue(dict, key.0) };
    if value.is_null() {
        return None;
    }
    let mut out: f64 = 0.0;
    let ok = unsafe {
        CFNumberGetValue(
            value,
            KCF_NUMBER_FLOAT64_TYPE,
            std::ptr::from_mut(&mut out).cast(),
        )
    };
    (ok != 0).then_some(out)
}

/// Read a CFNumber dictionary value as `i64`.
unsafe fn dict_i64(dict: CFTypeRef, key: &CFKey) -> Option<i64> {
    let value = unsafe { CFDictionaryGetValue(dict, key.0) };
    if value.is_null() {
        return None;
    }
    let mut out: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            value,
            KCF_NUMBER_SINT64_TYPE,
            std::ptr::from_mut(&mut out).cast(),
        )
    };
    (ok != 0).then_some(out)
}

/// Read a CFString dictionary value.
unsafe fn dict_string(dict: CFTypeRef, key: &CFKey) -> Option<String> {
    let value = unsafe { CFDictionaryGetValue(dict, key.0) };
    if value.is_null() {
        return None;
    }
    let mut buf = vec![0i8; 1024];
    let len = CFIndex::try_from(buf.len()).unwrap_or(1024);
    let ok = unsafe { CFStringGetCString(value, buf.as_mut_ptr(), len, KCF_STRING_ENCODING_UTF8) };
    if ok == 0 {
        return None;
    }
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c.cast_unsigned())
        .collect();
    String::from_utf8(bytes).ok()
}

/// Every on-screen window, front-to-back, minus desktop chrome. Needs no TCC
/// permission (window *titles* stay empty without it — fine, they're
/// decoration here). Errors only when the window server itself is
/// unreachable (an SSH session outside the GUI login).
pub fn list_windows() -> Result<Vec<WindowInfo>, CliError> {
    let list = unsafe {
        CGWindowListCopyWindowInfo(
            KCG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            KCG_NULL_WINDOW_ID,
        )
    };
    if list.is_null() {
        return Err(CliError::new(
            "cannot query the window server — screenshots need a GUI login session \
             (an SSH shell outside it can't see windows)",
        ));
    }
    let k_number = CFKey::new("kCGWindowNumber");
    let k_pid = CFKey::new("kCGWindowOwnerPID");
    let k_layer = CFKey::new("kCGWindowLayer");
    let k_owner = CFKey::new("kCGWindowOwnerName");
    let k_title = CFKey::new("kCGWindowName");
    let k_alpha = CFKey::new("kCGWindowAlpha");
    let k_bounds = CFKey::new("kCGWindowBounds");
    let k_width = CFKey::new("Width");
    let k_height = CFKey::new("Height");

    let mut windows = Vec::new();
    unsafe {
        for i in 0..CFArrayGetCount(list) {
            let dict = CFArrayGetValueAtIndex(list, i);
            if dict.is_null() {
                continue;
            }
            let (Some(number), Some(pid)) = (dict_i64(dict, &k_number), dict_i64(dict, &k_pid))
            else {
                continue;
            };
            let bounds = CFDictionaryGetValue(dict, k_bounds.0);
            let (width, height) = if bounds.is_null() {
                (0.0, 0.0)
            } else {
                (
                    dict_f64(bounds, &k_width).unwrap_or(0.0),
                    dict_f64(bounds, &k_height).unwrap_or(0.0),
                )
            };
            windows.push(WindowInfo {
                number: u32::try_from(number).unwrap_or(0),
                pid: i32::try_from(pid).unwrap_or(0),
                layer: i32::try_from(dict_i64(dict, &k_layer).unwrap_or(0)).unwrap_or(0),
                owner: dict_string(dict, &k_owner).unwrap_or_default(),
                title: dict_string(dict, &k_title),
                width,
                height,
                alpha: dict_f64(dict, &k_alpha).unwrap_or(1.0),
            });
        }
        CFRelease(list);
    }
    Ok(windows)
}

/// The pids whose running binary is `executable` (matched by full path, so
/// same-named binaries elsewhere never collide). Symlink spellings on either
/// side are unified by canonicalizing — the kernel reports the exec'd path,
/// which needn't be spelled like the project's (a `/tmp` vs `/private/tmp`
/// checkout, a symlinked dev directory).
pub fn pids_for_executable(executable: &Path) -> Result<Vec<i32>, CliError> {
    // -ww lifts the column width limit so long DerivedData paths survive.
    let out = process::capture("ps", &["-axww", "-o", "pid=,comm="], None)?;
    Ok(parse_ps_pids(&out, executable, |p| {
        std::fs::canonicalize(p).ok()
    }))
}

/// Parse `ps -o pid=,comm=` output, keeping pids whose command path names
/// `executable` under any spelling. A cheap file-name prefilter keeps the
/// `canonicalize` syscalls to the handful of same-named candidates.
fn parse_ps_pids(
    ps_output: &str,
    executable: &Path,
    canonicalize: impl Fn(&Path) -> Option<std::path::PathBuf>,
) -> Vec<i32> {
    let file_name = executable.file_name();
    let canonical = canonicalize(executable);
    ps_output
        .lines()
        .filter_map(|line| {
            let (pid, comm) = line.trim_start().split_once(' ')?;
            let comm = Path::new(comm);
            if comm.file_name() != file_name {
                return None;
            }
            let matches = comm == executable
                || Some(comm) == canonical.as_deref()
                || (canonical.is_some() && canonicalize(comm) == canonical);
            matches.then(|| pid.parse().ok())?
        })
        .collect()
}

/// Pick the window to capture among `windows` (the full front-to-back list)
/// for the given pids: capturable windows only, frontmost by default,
/// `index` (1-based, front-to-back) to override. `Ok` also carries how many
/// capturable windows the app has; `Err` is a complete, human-readable
/// reason — including the available windows when an index misses.
pub fn pick_window(
    windows: &[WindowInfo],
    pids: &[i32],
    index: Option<usize>,
) -> Result<(WindowInfo, usize), String> {
    let candidates: Vec<&WindowInfo> = windows
        .iter()
        .filter(|w| pids.contains(&w.pid) && w.capturable())
        .collect();
    if candidates.is_empty() {
        return Err("no on-screen window (still starting, or minimized?)".to_string());
    }
    match index {
        None => Ok((candidates[0].clone(), candidates.len())),
        Some(0) => Err("--window is 1-based (1 is the frontmost)".to_string()),
        Some(n) => candidates
            .get(n - 1)
            .map(|w| ((*w).clone(), candidates.len()))
            .ok_or_else(|| {
                let listing: Vec<String> = candidates
                    .iter()
                    .enumerate()
                    .map(|(i, w)| format!("{}: {}", i + 1, w.describe()))
                    .collect();
                format!(
                    "--window {n} is out of range — {} on-screen window(s): {}",
                    candidates.len(),
                    listing.join(", ")
                )
            }),
    }
}

/// Capture one window to a PNG: `screencapture -o -x -l<id>` (no shadow, no
/// sound, no focus steal — the window is captured wherever it is). The
/// caller preflights the Screen Recording permission; without it
/// screencapture "succeeds" with the wallpaper behind the window.
pub fn capture_window(window: u32, path: &Path) -> Result<(), CliError> {
    let dest = path.display().to_string();
    process::capture(
        "screencapture",
        &["-o", "-x", &format!("-l{window}"), &dest],
        None,
    )
    .map_err(|e| e.context("capturing the window"))?;
    if !path.exists() {
        return Err(CliError::new(format!(
            "screencapture reported success but wrote nothing to {dest}"
        )));
    }
    Ok(())
}

/// The error for a missing Screen Recording permission — actionable, and
/// aware that the grant goes to the hosting terminal app, not sweetpad.
pub fn permission_error() -> CliError {
    CliError::new(
        "screenshots need the Screen Recording permission — grant it to your terminal app \
         in System Settings → Privacy & Security → Screen Recording, then rerun (macOS \
         asks you to quit and reopen the terminal app for it to take effect)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(number: u32, pid: i32, layer: i32, alpha: f64, width: f64) -> WindowInfo {
        WindowInfo {
            number,
            pid,
            layer,
            owner: "App".into(),
            title: None,
            width,
            height: 600.0,
            alpha,
        }
    }

    #[test]
    fn ps_pids_match_the_full_executable_path() {
        let out = "  312 /Applications/Safari.app/Contents/MacOS/Safari\n\
                   4711 /Users/dev/Library/Developer/Xcode/DerivedData/My App-abc/Build/Products/Debug/My App.app/Contents/MacOS/My App\n\
                   4712 /usr/local/bin/My App\n\
                   9001 /Users/dev/other/My App.app/Contents/MacOS/My App\n";
        let exe = Path::new(
            "/Users/dev/Library/Developer/Xcode/DerivedData/My App-abc/Build/Products/Debug/My App.app/Contents/MacOS/My App",
        );
        // Paths with spaces survive; a same-named binary elsewhere doesn't match.
        assert_eq!(parse_ps_pids(out, exe, |_| None), vec![4711]);
        assert_eq!(
            parse_ps_pids(out, Path::new("/nonexistent"), |_| None),
            Vec::<i32>::new()
        );
    }

    #[test]
    fn ps_pids_match_across_symlink_spellings() {
        // The kernel reports the canonical path; the project knows the
        // symlinked spelling (or vice versa). Both directions resolve.
        let resolve = |p: &Path| {
            Some(std::path::PathBuf::from(
                p.display()
                    .to_string()
                    .replace("/tmp/", "/private/tmp/")
                    .replace("/private/private/", "/private/"),
            ))
        };
        let out = " 18248 /private/tmp/work/App.app/Contents/MacOS/App\n";
        let symlinked = Path::new("/tmp/work/App.app/Contents/MacOS/App");
        assert_eq!(parse_ps_pids(out, symlinked, resolve), vec![18248]);

        let out = " 18249 /tmp/work/App.app/Contents/MacOS/App\n";
        let canonical = Path::new("/private/tmp/work/App.app/Contents/MacOS/App");
        assert_eq!(parse_ps_pids(out, canonical, resolve), vec![18249]);
    }

    #[test]
    fn pick_prefers_the_frontmost_capturable_window() {
        let windows = vec![
            win(1, 99, 0, 1.0, 800.0),  // other app, in front
            win(2, 42, 25, 1.0, 800.0), // ours, but chrome layer
            win(3, 42, 0, 0.0, 800.0),  // ours, invisible
            win(4, 42, 0, 1.0, 1.0),    // ours, degenerate 1×600
            win(5, 42, 0, 1.0, 800.0),  // ours — the frontmost real window
            win(6, 42, 0, 1.0, 640.0),  // ours, behind it
        ];
        let (picked, count) = pick_window(&windows, &[42], None).unwrap();
        assert_eq!(picked.number, 5);
        assert_eq!(count, 2);
        // Explicit 1-based index, front-to-back over the capturable set.
        assert_eq!(pick_window(&windows, &[42], Some(2)).unwrap().0.number, 6);
    }

    #[test]
    fn pick_errors_are_actionable() {
        let windows = vec![win(5, 42, 0, 1.0, 800.0)];
        let err = pick_window(&windows, &[7], None).unwrap_err();
        assert!(err.contains("no on-screen window"), "{err}");
        let err = pick_window(&windows, &[42], Some(0)).unwrap_err();
        assert!(err.contains("1-based"), "{err}");
        let err = pick_window(&windows, &[42], Some(3)).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
        assert!(err.contains("1: 800×600"), "{err}");
    }

    #[test]
    fn window_listing_reaches_the_window_server() {
        // Runs wherever a GUI session exists (dev Macs, CI runners); in a
        // bare SSH session the explicit error path is the assertion instead.
        match list_windows() {
            Ok(windows) => {
                // The shape is sane: front-to-back entries with owners.
                for w in windows.iter().take(3) {
                    assert!(w.number > 0);
                }
            }
            Err(e) => assert!(e.to_string().contains("window server"), "{e}"),
        }
    }
}
