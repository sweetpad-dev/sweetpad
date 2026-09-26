//! Reading a running app's main thread out of `/usr/bin/sample` (CLI_DESIGN
//! §9r): capture a report, then classify the main thread as idle in its run
//! loop, blocked on a wait primitive, busy running code, or unclassified.
//!
//! `sample` writes one indented call tree per thread, each line a frame and the
//! number of samples that passed through it. Only what the verdict needs is
//! parsed — the thread headers, the frames under them, and the binary images
//! that tell the app's own code from the system's — and the rules are narrow on
//! purpose: a split they don't cover is `unclassified`, never a guess. The full
//! report stays on disk beside the verdict for everything else.

use std::path::Path;
use std::time::Duration;

use crate::cli::{CliError, ErrorKind};

/// "Nearly all" for `idle`. A responsive app still services the odd timer or
/// event while it is sampled, so a run-loop wait short of 100% is still idle;
/// below nine in ten the run loop is doing enough work that calling it idle
/// would hide it.
const IDLE_NUM: u64 = 9;
const IDLE_DEN: u64 = 10;

/// "Most" for `blocked` and `busy`: two thirds of the samples, so the named
/// state outweighs everything else combined twice over. A closer split is
/// reported as `unclassified` with its breakdown.
const MOST_NUM: u64 = 2;
const MOST_DEN: u64 = 3;

/// How many attributed frames the report keeps.
const TOP_FRAMES: usize = 5;

/// The image every syscall stub lives in; a sample that stopped there was in
/// the kernel rather than running code.
const KERNEL_IMAGE: &str = "libsystem_kernel.dylib";

/// Time `sample` gets past the sampling itself to symbolicate and write the
/// report, before it is killed. A large app takes seconds, not minutes.
const SYMBOLICATION_GRACE: Duration = Duration::from_secs(60);

/// One stack frame: the symbol and the image it's in, as `sample` printed them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    pub symbol: String,
    pub image: String,
}

#[derive(Debug)]
struct Node {
    frame: Frame,
    samples: u64,
    children: Vec<Node>,
}

#[derive(Debug)]
struct Thread {
    /// The header after the thread id — `DispatchQueue_1: com.apple.main-thread
    /// (serial)`, or `: <name>` for a named thread.
    header: String,
    samples: u64,
    roots: Vec<Node>,
}

impl Thread {
    fn is_main(&self) -> bool {
        self.header.contains("com.apple.main-thread") || self.header.contains("Main Thread")
    }

    /// The thread's name, when `sample` printed one (`Thread_123: name`).
    fn name(&self) -> Option<&str> {
        let name = self.header.strip_prefix(':')?.trim();
        let name = name
            .split("   DispatchQueue_")
            .next()
            .unwrap_or(name)
            .trim();
        (!name.is_empty()).then_some(name)
    }

    fn contains_symbol(&self, symbol: &str) -> bool {
        fn walk(nodes: &[Node], symbol: &str) -> bool {
            nodes
                .iter()
                .any(|n| n.frame.symbol == symbol || walk(&n.children, symbol))
        }
        walk(&self.roots, symbol)
    }
}

/// A parsed `sample` report: its threads and the images that are the app's
/// own code.
#[derive(Debug, Default)]
pub struct Report {
    threads: Vec<Thread>,
    /// Image names (as frames spell them) that belong to the sampled app.
    app_images: Vec<String>,
}

enum Section {
    Header,
    Graph,
    Between,
    Images,
}

/// Parse the parts of a `sample` report the verdict reads.
#[must_use]
pub fn parse(text: &str) -> Report {
    let mut report = Report::default();
    let mut process_path: Option<&str> = None;
    let mut images: Vec<&str> = Vec::new();
    let mut section = Section::Header;
    let mut current: Option<Thread> = None;
    let mut stack: Vec<Node> = Vec::new();

    for line in text.lines() {
        match section {
            Section::Header => {
                if let Some(path) = line.strip_prefix("Path:") {
                    process_path = Some(path.trim());
                } else if line.starts_with("Call graph:") {
                    section = Section::Graph;
                }
            }
            Section::Graph => {
                let Some((depth, samples, text)) = graph_line(line) else {
                    finish_thread(&mut report, &mut current, &mut stack);
                    section = Section::Between;
                    continue;
                };
                if depth == 0 {
                    finish_thread(&mut report, &mut current, &mut stack);
                    let header = text.strip_prefix("Thread_").map_or(text, |rest| {
                        rest.trim_start_matches(|c: char| c.is_ascii_digit())
                    });
                    current = Some(Thread {
                        header: header.trim_end().to_string(),
                        samples,
                        roots: Vec::new(),
                    });
                } else if let Some(thread) = current.as_mut() {
                    close_to(&mut stack, &mut thread.roots, depth - 1);
                    stack.push(Node {
                        frame: parse_frame(text),
                        samples,
                        children: Vec::new(),
                    });
                }
            }
            Section::Between => {
                if line.starts_with("Binary Images:") {
                    section = Section::Images;
                }
            }
            Section::Images => {
                if line.trim().is_empty() {
                    break;
                }
                if let Some(path) = image_path(line) {
                    images.push(path);
                }
            }
        }
    }
    finish_thread(&mut report, &mut current, &mut stack);
    report.app_images = app_images(process_path, &images);
    report
}

/// One call-graph line split into its tree depth, its sample count, and the
/// rest (a thread header at depth 0, a frame below it). The tree is drawn
/// with two columns per level out of `+`, `!`, `:`, `|` and spaces, after a
/// four-space margin.
fn graph_line(line: &str) -> Option<(usize, u64, &str)> {
    let body = line.strip_prefix("    ")?;
    let rest = body.trim_start_matches([' ', '+', '!', ':', '|']);
    let indent = body.len() - rest.len();
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let samples = rest[..digits].parse().ok()?;
    let text = rest[digits..].strip_prefix(' ')?;
    Some((indent / 2, samples, text))
}

/// `symbol  (in image) + 12  [0x…]  file.swift:3`, or `???  (in image)  load
/// address …` for a frame `sample` couldn't symbolicate.
fn parse_frame(text: &str) -> Frame {
    match text.split_once("  (in ") {
        Some((symbol, rest)) => Frame {
            symbol: symbol.trim().to_string(),
            image: rest.split(')').next().unwrap_or_default().to_string(),
        },
        None => Frame {
            symbol: text.split("  ").next().unwrap_or(text).trim().to_string(),
            image: String::new(),
        },
    }
}

/// Close the open frames deeper than `depth`, attaching each to its parent (or
/// to the thread's roots at the top).
fn close_to(stack: &mut Vec<Node>, roots: &mut Vec<Node>, depth: usize) {
    while stack.len() > depth {
        let Some(node) = stack.pop() else { break };
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => roots.push(node),
        }
    }
}

fn finish_thread(report: &mut Report, current: &mut Option<Thread>, stack: &mut Vec<Node>) {
    if let Some(mut thread) = current.take() {
        close_to(stack, &mut thread.roots, 0);
        report.threads.push(thread);
    }
    stack.clear();
}

/// A `Binary Images:` line's path, the text after the image UUID.
fn image_path(line: &str) -> Option<&str> {
    let (_, path) = line.split_once("> ")?;
    let path = path.trim();
    path.starts_with('/').then_some(path)
}

/// Which images are the app's own code. `sample` marks every image outside
/// the host OS with `+`, which inside a simulator is the whole iOS runtime too,
/// so the marker can't say it. The bundle can: an image inside the sampled
/// process's `.app` (its executable, `.debug.dylib`, embedded frameworks) is
/// the app's. Paths arrive with directories redacted (`/tmp/*/App.app/…`), so
/// the bundle is matched by name rather than by full path. A process outside
/// any bundle owns only its executable.
fn app_images(process_path: Option<&str>, images: &[&str]) -> Vec<String> {
    let Some(process_path) = process_path else {
        return Vec::new();
    };
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let bundle = process_path
        .split('/')
        .find(|c| {
            Path::new(c)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
        .map(|c| format!("/{c}/"));
    images
        .iter()
        .filter(|path| match &bundle {
            Some(bundle) => path.contains(bundle.as_str()),
            None => base(path) == base(process_path),
        })
        .map(|path| base(path))
        .collect()
}

/// What a thread was doing, as the verdict names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Blocked,
    Busy,
    Unclassified,
}

impl State {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Blocked => "blocked",
            State::Busy => "busy",
            State::Unclassified => "unclassified",
        }
    }
}

/// The wait primitives the `blocked` rule recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitKind {
    Mutex,
    Condition,
    RwLock,
    Semaphore,
    DispatchSync,
    DispatchGroup,
    DispatchOnce,
    UnfairLock,
    Ulock,
    Sleep,
    MachMsg,
}

impl WaitKind {
    /// The machine name, for `-o json`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WaitKind::Mutex => "mutex",
            WaitKind::Condition => "condition",
            WaitKind::RwLock => "rwlock",
            WaitKind::Semaphore => "semaphore",
            WaitKind::DispatchSync => "dispatchSync",
            WaitKind::DispatchGroup => "dispatchGroup",
            WaitKind::DispatchOnce => "dispatchOnce",
            WaitKind::UnfairLock => "unfairLock",
            WaitKind::Ulock => "ulock",
            WaitKind::Sleep => "sleep",
            WaitKind::MachMsg => "machMsg",
        }
    }

    /// What the thread waited on, for the human verdict.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            WaitKind::Mutex => "a mutex",
            WaitKind::Condition => "a condition variable",
            WaitKind::RwLock => "a read-write lock",
            WaitKind::Semaphore => "a semaphore",
            WaitKind::DispatchSync => "a dispatch_sync",
            WaitKind::DispatchGroup => "a dispatch group",
            WaitKind::DispatchOnce => "a dispatch_once",
            WaitKind::UnfairLock => "an os_unfair_lock",
            WaitKind::Ulock => "a lock",
            WaitKind::Sleep => "a sleep call",
            WaitKind::MachMsg => "a synchronous IPC reply",
        }
    }
}

/// A frame that says which wait a kernel stack is, and whether it is generic.
/// A generic marker names the wait only when nothing more specific sits below
/// it: `__ulock_wait2` is what an unfair lock, a dispatch group and
/// `pthread_join` all end in.
fn wait_marker(symbol: &str) -> Option<(WaitKind, bool)> {
    let specific = match symbol {
        "__psynch_mutexwait" | "_pthread_mutex_firstfit_lock_slow" | "_pthread_mutex_lock_slow" => {
            WaitKind::Mutex
        }
        "__psynch_cvwait" | "_pthread_cond_wait" => WaitKind::Condition,
        "__psynch_rw_rdlock" | "__psynch_rw_wrlock" | "__psynch_rw_yieldwrlock" => WaitKind::RwLock,
        "semaphore_wait_trap"
        | "semaphore_timedwait_trap"
        | "_dispatch_semaphore_wait_slow"
        | "_dispatch_sema4_wait"
        | "_dispatch_sema4_timedwait" => WaitKind::Semaphore,
        "__DISPATCH_WAIT_FOR_QUEUE__" | "_dispatch_sync_f_slow" | "_dispatch_sync_wait" => {
            WaitKind::DispatchSync
        }
        "_dispatch_group_wait_slow" => WaitKind::DispatchGroup,
        "_dispatch_once_wait" => WaitKind::DispatchOnce,
        "__semwait_signal" | "nanosleep" | "usleep" => WaitKind::Sleep,
        "_dispatch_mach_send_and_wait_for_reply"
        | "dispatch_mach_send_with_result_and_wait_for_reply"
        | "xpc_connection_send_message_with_reply_sync" => WaitKind::MachMsg,
        _ if symbol.contains("os_unfair_lock_lock") => WaitKind::UnfairLock,
        "__ulock_wait" | "__ulock_wait2" => return Some((WaitKind::Ulock, true)),
        _ if is_mach_msg(symbol) => return Some((WaitKind::MachMsg, true)),
        _ => return None,
    };
    Some((specific, false))
}

fn is_mach_msg(symbol: &str) -> bool {
    symbol.starts_with("mach_msg")
}

/// Where one frame hands control to client code: a wait marker found past
/// here belongs to whatever called the callout, not to the stack's top.
fn is_callout(symbol: &str) -> bool {
    symbol.starts_with("_dispatch_client_callout") || symbol.starts_with("__CFRUNLOOP_IS_")
}

/// Frames on every main-thread stack that name nothing: the program entry
/// points and Swift's compiler-generated thunks.
fn is_uninformative(symbol: &str) -> bool {
    matches!(
        symbol,
        "main" | "start" | "__debug_main_executable_dylib_entry_point"
    ) || symbol.ends_with(".$main()")
        || symbol.starts_with("thunk for ")
        || symbol.starts_with("reabstraction thunk")
}

/// What one sample's stack was doing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Class {
    /// Parked in the run loop's `mach_msg`, waiting for the next event.
    RunLoop,
    /// In the kernel on a recognized wait primitive; `marker` is the frame
    /// that named it.
    Wait { kind: WaitKind, marker: String },
    /// Executing user-space code.
    Running,
    /// In the kernel on anything else — a `read`, an `open` — which none of
    /// the rules claim.
    Syscall,
}

fn classify(stack: &[&Frame], app_images: &[String]) -> Class {
    let Some(top) = stack.last() else {
        return Class::Syscall;
    };
    if top.image != KERNEL_IMAGE {
        return Class::Running;
    }
    // The run-loop wait is exactly `mach_msg` stubs directly under the run
    // loop's port service — a `mach_msg` anywhere else is someone waiting on
    // a reply.
    let below_kernel = stack.iter().rev().find(|f| f.image != KERNEL_IMAGE);
    let kernel_is_mach_msg = stack
        .iter()
        .rev()
        .take_while(|f| f.image == KERNEL_IMAGE)
        .all(|f| is_mach_msg(&f.symbol));
    if kernel_is_mach_msg
        && below_kernel.is_some_and(|f| {
            f.symbol == "__CFRunLoopServiceMachPort" || f.symbol == "__CFRunLoopRun"
        })
    {
        return Class::RunLoop;
    }
    let mut generic = None;
    for frame in stack.iter().rev() {
        if app_images.contains(&frame.image) || is_callout(&frame.symbol) {
            break;
        }
        match wait_marker(&frame.symbol) {
            Some((kind, false)) => {
                return Class::Wait {
                    kind,
                    marker: frame.symbol.clone(),
                };
            }
            Some((kind, true)) => {
                generic.get_or_insert((kind, frame.symbol.clone()));
            }
            None => {}
        }
    }
    generic.map_or(Class::Syscall, |(kind, marker)| Class::Wait {
        kind,
        marker,
    })
}

/// A frame and how many samples were attributed to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopFrame {
    pub frame: Frame,
    pub samples: u64,
}

/// What the main thread was blocked on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub kind: WaitKind,
    /// The frame that identified the wait, e.g. `__psynch_mutexwait`.
    pub symbol: String,
    /// The first app frame beneath the wait on its heaviest stack, when the
    /// app's own code is what waited.
    pub caller: Option<Frame>,
    pub samples: u64,
}

/// Sample counts per class, over the whole main thread.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Breakdown {
    pub run_loop: u64,
    pub waiting: u64,
    pub running: u64,
    pub syscall: u64,
}

/// The verdict on the main thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainThread {
    pub state: State,
    pub samples: u64,
    pub breakdown: Breakdown,
    /// The heaviest frames among the samples behind the verdict (all of them
    /// when unclassified): each sample goes to its innermost app frame, or to
    /// the frame it stopped in when no app code was on the stack.
    pub top_frames: Vec<TopFrame>,
    /// Set when blocked.
    pub wait: Option<Wait>,
}

/// Something outside the main-thread verdict worth saying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flag {
    /// AppKit caught an Objective-C exception and carried on. HIServices
    /// records it by parking a thread named `HIE: …` in a function named
    /// `SOME_OTHER_THREAD_SWALLOWED_AT_LEAST_ONE_EXCEPTION`; `thread` is that
    /// name, which carries the time of the first one.
    SwallowedException { thread: String },
}

/// The whole reading of one report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// Unclassified with no samples when the report held no call graph.
    pub main_thread: MainThread,
    pub flags: Vec<Flag>,
}

/// A stack with a positive self weight: the frames root-to-top, and how many
/// samples stopped exactly there.
fn for_each_stack<'a>(
    nodes: &'a [Node],
    path: &mut Vec<&'a Frame>,
    f: &mut impl FnMut(&[&'a Frame], u64),
) {
    for node in nodes {
        path.push(&node.frame);
        let inner: u64 = node.children.iter().map(|c| c.samples).sum();
        let own = node.samples.saturating_sub(inner);
        if own > 0 {
            f(path, own);
        }
        for_each_stack(&node.children, path, f);
        path.pop();
    }
}

/// The app's own code nearest the top of a stack, past the entry points and
/// thunks every main-thread stack carries.
fn innermost_app_frame<'a>(stack: &[&'a Frame], app_images: &[String]) -> Option<&'a Frame> {
    stack
        .iter()
        .rev()
        .find(|f| app_images.contains(&f.image) && !is_uninformative(&f.symbol))
        .copied()
}

/// The frame a stack's samples are credited to: its innermost app frame, else
/// the frame it stopped in.
fn attribution(stack: &[&Frame], app_images: &[String]) -> Frame {
    innermost_app_frame(stack, app_images)
        .or(stack.last().copied())
        .map_or_else(Frame::default, Frame::clone)
}

fn at_least(part: u64, total: u64, num: u64, den: u64) -> bool {
    total > 0 && part * den >= total * num
}

/// Classify the main thread and collect the flags.
#[must_use]
pub fn analyze(report: &Report) -> Analysis {
    let mut flags = Vec::new();
    for thread in &report.threads {
        let named = thread.name().filter(|n| n.starts_with("HIE:"));
        if named.is_some()
            || thread.contains_symbol("SOME_OTHER_THREAD_SWALLOWED_AT_LEAST_ONE_EXCEPTION")
        {
            flags.push(Flag::SwallowedException {
                thread: named
                    .or_else(|| thread.name())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }
    let main = report
        .threads
        .iter()
        .find(|t| t.is_main())
        .or(report.threads.first());
    Analysis {
        main_thread: main.map_or_else(
            || MainThread {
                state: State::Unclassified,
                samples: 0,
                breakdown: Breakdown::default(),
                top_frames: Vec::new(),
                wait: None,
            },
            |t| main_thread(t, &report.app_images),
        ),
        flags,
    }
}

fn main_thread(thread: &Thread, app_images: &[String]) -> MainThread {
    let mut stacks: Vec<(Vec<&Frame>, u64, Class)> = Vec::new();
    for_each_stack(&thread.roots, &mut Vec::new(), &mut |stack, samples| {
        stacks.push((stack.to_vec(), samples, classify(stack, app_images)));
    });

    let mut breakdown = Breakdown::default();
    for (_, samples, class) in &stacks {
        match class {
            Class::RunLoop => breakdown.run_loop += samples,
            Class::Wait { .. } => breakdown.waiting += samples,
            Class::Running => breakdown.running += samples,
            Class::Syscall => breakdown.syscall += samples,
        }
    }

    let total = thread.samples;
    let state = if at_least(breakdown.run_loop, total, IDLE_NUM, IDLE_DEN) {
        State::Idle
    } else if at_least(breakdown.waiting, total, MOST_NUM, MOST_DEN) {
        State::Blocked
    } else if at_least(breakdown.running, total, MOST_NUM, MOST_DEN) {
        State::Busy
    } else {
        State::Unclassified
    };

    // The samples behind the verdict, credited to frames in first-seen order
    // so the stable sort below keeps the report's order among equal counts.
    let behind = |class: &Class| match state {
        State::Idle => *class == Class::RunLoop,
        State::Blocked => matches!(class, Class::Wait { .. }),
        State::Busy => *class == Class::Running,
        State::Unclassified => true,
    };
    let mut top_frames: Vec<TopFrame> = Vec::new();
    for (stack, samples, _) in stacks.iter().filter(|(_, _, class)| behind(class)) {
        let frame = attribution(stack, app_images);
        match top_frames.iter_mut().find(|t| t.frame == frame) {
            Some(top) => top.samples += samples,
            None => top_frames.push(TopFrame {
                frame,
                samples: *samples,
            }),
        }
    }
    top_frames.sort_by(|a, b| b.samples.cmp(&a.samples));
    top_frames.truncate(TOP_FRAMES);

    MainThread {
        state,
        samples: total,
        breakdown,
        top_frames,
        wait: (state == State::Blocked)
            .then(|| heaviest_wait(&stacks, app_images))
            .flatten(),
    }
}

/// The wait kind with the most samples, described by its heaviest stack.
fn heaviest_wait(stacks: &[(Vec<&Frame>, u64, Class)], app_images: &[String]) -> Option<Wait> {
    let mut kinds: Vec<(WaitKind, u64)> = Vec::new();
    for (_, samples, class) in stacks {
        if let Class::Wait { kind, .. } = class {
            match kinds.iter_mut().find(|(k, _)| k == kind) {
                Some((_, n)) => *n += samples,
                None => kinds.push((*kind, *samples)),
            }
        }
    }
    let (kind, samples) = kinds.into_iter().max_by_key(|(_, n)| *n)?;
    let (stack, marker) = stacks
        .iter()
        .filter_map(|(stack, n, class)| match class {
            Class::Wait { kind: k, marker } if *k == kind => Some((stack, marker, *n)),
            _ => None,
        })
        .max_by_key(|(_, _, n)| *n)
        .map(|(stack, marker, _)| (stack, marker))?;
    Some(Wait {
        kind,
        symbol: marker.clone(),
        caller: innermost_app_frame(stack, app_images).cloned(),
        samples,
    })
}

/// Sample `pid` for `seconds` into `path` and return the report's text.
/// Bounded: `sample` gets the sampling time plus [`SYMBOLICATION_GRACE`], and
/// is killed past that.
pub fn capture(pid: i32, seconds: u64, path: &Path) -> Result<String, CliError> {
    use std::io::Read as _;

    let mut child = std::process::Command::new("/usr/bin/sample")
        .arg(pid.to_string())
        .arg(seconds.to_string())
        .arg("-file")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let err = CliError::new(format!("failed to run '/usr/bin/sample': {e}"));
            if e.kind() == std::io::ErrorKind::NotFound {
                err.kind(ErrorKind::ToolMissing)
            } else {
                err
            }
        })?;
    let slot = crate::cli::signals::register_child(child.id());
    let limit = Duration::from_secs(seconds) + SYMBOLICATION_GRACE;
    let timed_out = super::wait_with_timeout(&mut child, limit);
    if timed_out {
        let _ = child.kill();
    }
    let status = child.wait();
    crate::cli::signals::unregister_child(slot);
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if timed_out {
        return Err(CliError::new(format!(
            "'sample' was still running {}s after it started and was stopped",
            limit.as_secs()
        )));
    }
    if !status.is_ok_and(|s| s.success()) {
        return Err(CliError::new(sample_failure(&stderr)).context(format!("sampling pid {pid}")));
    }
    std::fs::read_to_string(path).map_err(|e| {
        CliError::new(format!(
            "'sample' finished but its report {} can't be read: {e}",
            path.display()
        ))
    })
}

/// `sample`'s own reason for failing, without its `sample[123]: ` prefix.
fn sample_failure(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("'sample' exited with an error");
    match line.split_once("]: ") {
        Some((prefix, reason)) if prefix.starts_with("sample[") => reason.trim().to_string(),
        _ => line.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `sample` reports from a scratch SwiftUI app driven into each state
    /// by a launch environment variable, trimmed to the header, the main
    /// thread (plus one ordinary thread, and the telltale thread where there is
    /// one), and the app's binary images.
    fn fixture(name: &str) -> Analysis {
        let path = format!("{}/fixtures/sample/{name}.txt", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        analyze(&parse(&text))
    }

    fn main_of(name: &str) -> MainThread {
        fixture(name).main_thread
    }

    fn frame(symbol: &str, image: &str) -> Frame {
        Frame {
            symbol: symbol.to_string(),
            image: image.to_string(),
        }
    }

    #[test]
    fn graph_lines_carry_depth_and_count() {
        assert_eq!(
            graph_line(
                "    2611 Thread_19566204   DispatchQueue_1: com.apple.main-thread  (serial)"
            ),
            Some((
                0,
                2611,
                "Thread_19566204   DispatchQueue_1: com.apple.main-thread  (serial)"
            ))
        );
        assert_eq!(
            graph_line("    +   ! 882 static Probe.isPrime(_:)  (in App) + 1  [0x1]"),
            Some((3, 882, "static Probe.isPrime(_:)  (in App) + 1  [0x1]"))
        );
        assert_eq!(graph_line(""), None);
        assert_eq!(
            graph_line("Total number in stack (recursive counted multiple, when >=5):"),
            None
        );
    }

    #[test]
    fn frames_split_symbol_from_image() {
        assert_eq!(
            parse_frame(
                "closure #1 in static Probe.start()  (in SampleProbe.debug.dylib) + 36  [0x1]  \
                 SampleProbeApp.swift:21"
            ),
            frame(
                "closure #1 in static Probe.start()",
                "SampleProbe.debug.dylib"
            )
        );
        assert_eq!(
            parse_frame("???  (in AE)  load address 0x1a4759000 + 0xaf9c  [0x1a4763f9c]"),
            frame("???", "AE")
        );
    }

    #[test]
    fn app_images_are_the_ones_inside_the_bundle() {
        let images = [
            "/tmp/*/SampleProbe.app/Contents/MacOS/SampleProbe",
            "/tmp/*/SampleProbe.app/Contents/MacOS/SampleProbe.debug.dylib",
            "/usr/lib/system/libsystem_kernel.dylib",
            "/Volumes/*/UIKitCore",
        ];
        assert_eq!(
            app_images(
                Some("/private/tmp/*/SampleProbe.app/Contents/MacOS/SampleProbe"),
                &images
            ),
            ["SampleProbe", "SampleProbe.debug.dylib"]
        );
        // Outside a bundle only the executable is the program's own.
        assert_eq!(
            app_images(
                Some("/usr/local/bin/tool"),
                &["/usr/local/bin/tool", "/usr/lib/libz.dylib"]
            ),
            ["tool"]
        );
        assert!(app_images(None, &images).is_empty());
    }

    #[test]
    fn an_idle_mac_app_is_waiting_in_its_run_loop() {
        let main = main_of("mac-idle");
        assert_eq!(main.state, State::Idle);
        assert_eq!(main.samples, 2611);
        assert_eq!(main.breakdown.run_loop, 2611);
        assert!(main.wait.is_none());
    }

    #[test]
    fn an_idle_simulator_app_is_waiting_in_uikits_run_loop() {
        // `UIApplicationMain` → `GSEventRunModal` rather than AppKit's
        // `-[NSApplication run]`, and every runtime image carries `+`.
        let main = main_of("sim-idle");
        assert_eq!(main.state, State::Idle);
        assert_eq!(main.breakdown.run_loop, main.samples);
    }

    #[test]
    fn a_modal_alert_is_idle_in_its_nested_run_loop() {
        // The "reopen windows?" alert after a crash: the main thread waits for
        // events inside `-[NSAlert runModal]`, under an Apple Event handler.
        let main = main_of("mac-modal-alert");
        assert_eq!(main.state, State::Idle);
        assert_eq!(main.breakdown.run_loop, 2595);
        assert_eq!(main.breakdown.waiting, 1, "one sync LaunchServices reply");
    }

    #[test]
    fn a_spinning_main_thread_is_busy_in_the_apps_own_frames() {
        let main = main_of("mac-busy");
        assert_eq!(main.state, State::Busy);
        assert_eq!(main.breakdown.running, 2530);
        assert_eq!(
            main.top_frames,
            [
                TopFrame {
                    frame: frame("static Probe.isPrime(_:)", "SampleProbe.debug.dylib"),
                    samples: 2522,
                },
                TopFrame {
                    frame: frame("static Probe.crunchForever()", "SampleProbe.debug.dylib"),
                    samples: 8,
                },
            ]
        );
    }

    fn assert_blocked(name: &str, kind: WaitKind, symbol: &str, caller: &str) {
        let main = main_of(name);
        assert_eq!(main.state, State::Blocked, "{name}");
        let wait = main.wait.expect("a wait");
        assert_eq!(wait.kind, kind, "{name}");
        assert_eq!(wait.symbol, symbol, "{name}");
        assert_eq!(
            wait.caller,
            Some(frame(caller, "SampleProbe.debug.dylib")),
            "{name}"
        );
        assert_eq!(wait.samples, main.samples, "{name}");
    }

    #[test]
    fn a_main_thread_on_a_semaphore_is_blocked() {
        assert_blocked(
            "mac-semaphore",
            WaitKind::Semaphore,
            "semaphore_wait_trap",
            "static Probe.waitOnSemaphore()",
        );
    }

    #[test]
    fn a_main_thread_on_a_held_mutex_is_blocked() {
        assert_blocked(
            "mac-mutex",
            WaitKind::Mutex,
            "__psynch_mutexwait",
            "static Probe.waitOnMutex()",
        );
    }

    #[test]
    fn an_unfair_lock_is_named_over_the_ulock_beneath_it() {
        assert_blocked(
            "mac-unfair-lock",
            WaitKind::UnfairLock,
            "_os_unfair_lock_lock_slow",
            "static Probe.waitOnUnfairLock()",
        );
    }

    #[test]
    fn a_main_thread_on_a_condition_is_blocked() {
        assert_blocked(
            "mac-condition",
            WaitKind::Condition,
            "__psynch_cvwait",
            "static Probe.waitOnCondition()",
        );
    }

    #[test]
    fn a_dispatch_sync_onto_a_stuck_queue_is_blocked() {
        // The wait ends in `kevent_id`, not a ulock, so only the dispatch
        // frames beneath it say what it is.
        assert_blocked(
            "mac-dispatch-sync",
            WaitKind::DispatchSync,
            "__DISPATCH_WAIT_FOR_QUEUE__",
            "static Probe.syncOntoBlockedQueue()",
        );
    }

    #[test]
    fn a_swallowed_exception_is_flagged_by_its_telltale_thread() {
        let analysis = fixture("mac-swallowed-exception");
        assert_eq!(
            analysis.flags,
            [Flag::SwallowedException {
                thread: "HIE: M_ 8bb450dcefffa491 2026-09-26 17:50:23.629".to_string(),
            }]
        );
        // The app carried on past it, which is the trap: it looks fine.
        assert_eq!(analysis.main_thread.state, State::Idle);
        assert!(fixture("mac-idle").flags.is_empty());
    }

    fn stack(frames: &[(&str, &str)]) -> Vec<Frame> {
        frames.iter().map(|(s, i)| frame(s, i)).collect()
    }

    #[test]
    fn a_mach_msg_outside_the_run_loop_is_a_wait_for_a_reply() {
        let frames = stack(&[
            ("-[Model load]", "App"),
            (
                "xpc_connection_send_message_with_reply_sync",
                "libxpc.dylib",
            ),
            (
                "_dispatch_mach_send_and_wait_for_reply",
                "libdispatch.dylib",
            ),
            ("mach_msg", KERNEL_IMAGE),
            ("mach_msg2_trap", KERNEL_IMAGE),
        ]);
        let refs: Vec<&Frame> = frames.iter().collect();
        assert_eq!(
            classify(&refs, &["App".to_string()]),
            Class::Wait {
                kind: WaitKind::MachMsg,
                marker: "_dispatch_mach_send_and_wait_for_reply".to_string(),
            }
        );
    }

    #[test]
    fn an_unrecognized_syscall_is_not_claimed_by_any_rule() {
        let frames = stack(&[("-[Model load]", "App"), ("read", KERNEL_IMAGE)]);
        let refs: Vec<&Frame> = frames.iter().collect();
        assert_eq!(classify(&refs, &["App".to_string()]), Class::Syscall);
    }

    #[test]
    fn a_split_short_of_the_thresholds_is_unclassified() {
        let text = "\
Path:            /tmp/*/App.app/Contents/MacOS/App
Call graph:
    100 Thread_1   DispatchQueue_1: com.apple.main-thread  (serial)
    + 100 main  (in App) + 1  [0x1]
    +   50 __CFRunLoopServiceMachPort  (in CoreFoundation) + 1  [0x1]
    +   ! 50 mach_msg2_trap  (in libsystem_kernel.dylib) + 1  [0x1]
    +   50 -[Model work]  (in App) + 1  [0x1]

Binary Images:
       0x1 -        0x2 +com.example.App (1.0) <UUID> /tmp/*/App.app/Contents/MacOS/App
";
        let main = analyze(&parse(text)).main_thread;
        assert_eq!(main.state, State::Unclassified);
        assert_eq!(
            main.breakdown,
            Breakdown {
                run_loop: 50,
                waiting: 0,
                running: 50,
                syscall: 0,
            }
        );
        assert!(main.wait.is_none());
    }

    #[test]
    fn a_report_without_a_call_graph_is_unclassified() {
        let main = analyze(&parse("garbage\n")).main_thread;
        assert_eq!(main.state, State::Unclassified);
        assert_eq!(main.samples, 0);
    }

    #[test]
    fn samples_own_reason_is_kept_without_its_prefix() {
        assert_eq!(
            sample_failure(
                "sample[79130]: sample cannot examine process 1 (launchd) because you do not \
                 have appropriate privileges to examine it; try running with `sudo`.\n"
            ),
            "sample cannot examine process 1 (launchd) because you do not have appropriate \
             privileges to examine it; try running with `sudo`."
        );
        assert_eq!(sample_failure(""), "'sample' exited with an error");
    }
}
