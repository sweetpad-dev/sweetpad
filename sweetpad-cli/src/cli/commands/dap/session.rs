//! One debug session: the editor on one side, lldb-dap on the other, and the
//! build and launch in between (CLI_DESIGN §9u).
//!
//! lldb-dap starts with the editor's `initialize`. The editor's `launch` and
//! `attach` never reach it as sent: the adapter builds and starts the app,
//! then sends lldb-dap the request that reaches the process, reusing the
//! editor's `seq` so lldb-dap's reply already answers the editor's request.
//! `disconnect` is forwarded and then stops the app and its log stream.
//! Everything else passes through.
//!
//! Everything sent to the editor carries the adapter's own `seq`, since the
//! adapter injects messages of its own (build output, app logs) and lldb-dap
//! numbers nothing. Reverse requests from lldb-dap (`runInTerminal`) are
//! renumbered on the way out, and the editor's replies mapped back.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sweetpad_core::framing;

use super::stdio;
use crate::cli::commands::app::adapter::{self, AppSession, Cancel, LaunchConfig, Prepared, Sink};
use crate::cli::output::Output;
use crate::cli::{CliError, CliResult, CommandResult, Context, Rendered, process};

/// The id of the build-and-launch progress indicator, which the editor may
/// cancel.
const LAUNCH_PROGRESS: &str = "sweetpad-launch";

/// How often the progress indicator may change, so a build compiling hundreds
/// of files doesn't flood the editor with updates.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// What the session's loop waits on.
enum Inbound {
    Editor(Value),
    EditorClosed,
    Adapter(Value),
    AdapterClosed,
}

/// Serve one debug session over stdio, until the editor or lldb-dap goes away.
pub(super) fn serve(ctx: &mut Context) -> CommandResult {
    // A debug session has no terminal: never prompt, animate or color, and
    // keep the human forms, which become the debug console's lines.
    ctx.global.non_interactive = true;
    ctx.global.no_color = true;
    ctx.global.output = None;
    ctx.global.json = false;
    ctx.out = Output::new(&ctx.global);
    let io = stdio::take_over().map_err(|e| CliError::new(format!("taking over stdio: {e}")))?;
    let log = TrafficLog::open();
    let editor = Arc::new(Editor::new(io.output, log.clone()));
    capture(io.stdout, &editor, "stdout");
    capture(io.stderr, &editor, "console");
    let cancel = Arc::new(Cancel::default());
    let (tx, rx) = mpsc::channel();
    read_editor(io.input, tx.clone(), Arc::clone(&cancel), log.clone());
    let mut session = Session {
        ctx,
        editor,
        tx,
        cancel,
        log,
        adapter: None,
        initializing: None,
        debuggee: None,
        start: None,
        reverse: HashMap::new(),
        disconnecting: None,
    };
    session.run(&rx);
    Ok(Rendered::Streamed)
}

/// `SWEETPAD_DAP_LOG`: a file that records every message both ways, as
/// `SWEETPAD_BSP_LOG` does for the BSP server.
#[derive(Clone, Default)]
struct TrafficLog(Option<Arc<Mutex<File>>>);

impl TrafficLog {
    fn open() -> Self {
        let Some(path) = std::env::var_os("SWEETPAD_DAP_LOG").filter(|p| !p.is_empty()) else {
            return Self(None);
        };
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok();
        Self(file.map(|f| Arc::new(Mutex::new(f))))
    }

    fn record(&self, direction: &str, body: &str) {
        let Some(file) = &self.0 else {
            return;
        };
        if let Ok(mut file) = file.lock() {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let _ = writeln!(file, "[{ms}] {direction} {body}");
        }
    }
}

/// The editor's end: every message to it goes through here, so each gets the
/// next `seq` in the order it was written.
struct Editor {
    out: Mutex<EditorOut>,
    log: TrafficLog,
    /// From the editor's `initialize`: whether lines and columns count from 1,
    /// and whether it shows progress.
    lines_from_one: AtomicBool,
    columns_from_one: AtomicBool,
    progress: AtomicBool,
    last_step: Mutex<Option<Instant>>,
}

struct EditorOut {
    file: File,
    seq: i64,
    /// False once a write failed or the session ended; later writes are
    /// dropped.
    open: bool,
}

impl Editor {
    fn new(file: File, log: TrafficLog) -> Self {
        Editor {
            out: Mutex::new(EditorOut {
                file,
                seq: 0,
                open: true,
            }),
            log,
            lines_from_one: AtomicBool::new(true),
            columns_from_one: AtomicBool::new(true),
            progress: AtomicBool::new(false),
            last_step: Mutex::new(None),
        }
    }

    /// Send `message` with the next `seq`, which is returned.
    fn send(&self, mut message: Value) -> i64 {
        let mut out = self
            .out
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        out.seq += 1;
        let seq = out.seq;
        message["seq"] = json!(seq);
        let body = message.to_string();
        if out.open && framing::write_message(&mut out.file, &body).is_err() {
            out.open = false;
        }
        self.log.record("to editor", &body);
        seq
    }

    fn close(&self) {
        self.out
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .open = false;
    }

    fn event(&self, event: &str, body: &Value) {
        self.send(json!({ "type": "event", "event": event, "body": body }));
    }

    /// Answer `request`. A failure carries the message as `body.error` too,
    /// which editors show to the user rather than only logging.
    fn respond(&self, request: &Value, outcome: Result<Option<Value>, String>) {
        let mut response = json!({
            "type": "response",
            "request_seq": request["seq"],
            "command": request["command"],
            "success": outcome.is_ok(),
        });
        match outcome {
            Ok(Some(body)) => response["body"] = body,
            Ok(None) => {}
            Err(message) => {
                response["body"] = json!({
                    "error": { "id": 1, "format": message, "showUser": true },
                });
                response["message"] = json!(message);
            }
        }
        self.send(response);
    }

    fn output(&self, category: &str, text: &str) {
        self.event(
            "output",
            &json!({ "category": category, "output": format!("{text}\n") }),
        );
    }

    fn progress_start(&self, title: &str) {
        if self.progress.load(Ordering::Relaxed) {
            self.event(
                "progressStart",
                &json!({ "progressId": LAUNCH_PROGRESS, "title": title, "cancellable": true }),
            );
        }
    }

    fn progress_end(&self, message: &str) {
        if self.progress.load(Ordering::Relaxed) {
            self.event(
                "progressEnd",
                &json!({ "progressId": LAUNCH_PROGRESS, "message": message }),
            );
        }
    }

    /// A line or column from the build log, which counts from 1, in the
    /// editor's convention.
    fn position(value: u64, from_one: &AtomicBool) -> u64 {
        if from_one.load(Ordering::Relaxed) {
            value
        } else {
            value.saturating_sub(1)
        }
    }
}

impl Sink for Editor {
    fn console(&self, text: &str) {
        self.output("console", text);
    }

    fn diagnostic(&self, severity: &str, location: Option<&str>, text: &str) {
        let category = if severity == "error" {
            "stderr"
        } else {
            "console"
        };
        let mut body = json!({ "category": category, "output": format!("{text}\n") });
        if let Some((path, line, column)) = location.and_then(parse_location) {
            let name = std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
            body["source"] = json!({ "name": name, "path": path });
            body["line"] = json!(Self::position(line, &self.lines_from_one));
            if let Some(column) = column {
                body["column"] = json!(Self::position(column, &self.columns_from_one));
            }
        }
        self.event("output", &body);
    }

    fn step(&self, message: &str) {
        if !self.progress.load(Ordering::Relaxed) {
            return;
        }
        let mut last = self
            .last_step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < PROGRESS_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
        drop(last);
        self.event(
            "progressUpdate",
            &json!({ "progressId": LAUNCH_PROGRESS, "message": message }),
        );
    }
}

/// Split a build log's `file:line:col` (or `file:line`) into its parts.
fn parse_location(location: &str) -> Option<(&str, u64, Option<u64>)> {
    let (rest, last) = location.rsplit_once(':')?;
    let last: u64 = last.parse().ok()?;
    if let Some((path, line)) = rest.rsplit_once(':')
        && let Ok(line) = line.parse()
    {
        return Some((path, line, Some(last)));
    }
    Some((rest, last, None))
}

/// Turn each line written to a captured fd into an `output` event.
fn capture(pipe: File, editor: &Arc<Editor>, category: &'static str) {
    let editor = Arc::clone(editor);
    std::thread::spawn(move || {
        process::read_lines_lossy(pipe, &mut |line: &str| editor.output(category, line));
    });
}

/// Read the editor's messages onto the session's queue. A request that ends
/// the session also cancels a launch in flight from here, since the session's
/// own thread is busy building it.
fn read_editor(input: File, tx: Sender<Inbound>, cancel: Arc<Cancel>, log: TrafficLog) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(input);
        loop {
            let Ok(Some(body)) = framing::read_message(&mut reader) else {
                cancel.fire();
                let _ = tx.send(Inbound::EditorClosed);
                return;
            };
            log.record("from editor", &body);
            let Ok(message) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            if ends_launch(&message) {
                cancel.fire();
            }
            if tx.send(Inbound::Editor(message)).is_err() {
                return;
            }
        }
    });
}

/// Whether the editor wants the launch to stop: it is ending the session, or
/// cancelled the launch's progress.
fn ends_launch(message: &Value) -> bool {
    if message["type"] != "request" {
        return false;
    }
    match message["command"].as_str() {
        Some("disconnect" | "terminate") => true,
        Some("cancel") => message["arguments"]["progressId"] == LAUNCH_PROGRESS,
        _ => false,
    }
}

/// The lldb-dap child.
struct Adapter {
    child: Child,
    stdin: ChildStdin,
    slot: Option<usize>,
}

impl Adapter {
    /// Start lldb-dap, reading its messages onto the session's queue. Its
    /// stderr is the adapter's, so what it reports shows in the console.
    fn spawn(tx: Sender<Inbound>, log: TrafficLog) -> Result<Self, CliError> {
        let (program, args) = super::lldb_dap_command();
        let mut child = Command::new(&program)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| process::spawn_error(&program, &e).context("starting lldb-dap"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(CliError::new("lldb-dap started without pipes"));
        };
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(body)) = framing::read_message(&mut reader) {
                log.record("from lldb-dap", &body);
                if let Ok(message) = serde_json::from_str::<Value>(&body)
                    && tx.send(Inbound::Adapter(message)).is_err()
                {
                    return;
                }
            }
            let _ = tx.send(Inbound::AdapterClosed);
        });
        let slot = crate::cli::signals::register_child(child.id());
        Ok(Adapter { child, stdin, slot })
    }

    fn send(&mut self, message: &Value, log: &TrafficLog) {
        let body = message.to_string();
        log.record("to lldb-dap", &body);
        let _ = framing::write_message(&mut self.stdin, &body);
    }

    /// Close lldb-dap's input and give it a moment to exit before killing it.
    fn stop(self) {
        let Adapter {
            mut child,
            stdin,
            slot,
        } = self;
        drop(stdin);
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        crate::cli::signals::unregister_child(slot);
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The app the session debugs, once lldb-dap has been asked to reach it.
struct Debuggee {
    /// The editor's request, whose reply lldb-dap gives under the command it
    /// was sent instead.
    request_seq: Value,
    editor_command: String,
    forwarded: &'static str,
    /// The editor launched it, rather than attaching to it: a launched app
    /// stops with the session unless the editor says otherwise.
    launched: bool,
    app: Option<AppSession>,
}

struct Session<'a> {
    ctx: &'a mut Context,
    editor: Arc<Editor>,
    tx: Sender<Inbound>,
    cancel: Arc<Cancel>,
    log: TrafficLog,
    adapter: Option<Adapter>,
    /// The editor's `initialize`, until lldb-dap answers it.
    initializing: Option<Value>,
    debuggee: Option<Debuggee>,
    /// Starts the app once lldb-dap has its configuration (Designed for iPad).
    start: Option<Box<dyn FnOnce() -> CliResult + Send>>,
    /// Reverse requests in flight: the `seq` the editor saw, to lldb-dap's.
    reverse: HashMap<i64, Value>,
    /// The editor's `disconnect`, until lldb-dap answers it, and whether the
    /// app stops.
    disconnecting: Option<(Value, bool)>,
}

impl Session<'_> {
    fn run(&mut self, rx: &Receiver<Inbound>) {
        while let Ok(inbound) = rx.recv() {
            match inbound {
                Inbound::Editor(message) => {
                    if self.editor_message(message) {
                        break;
                    }
                }
                Inbound::Adapter(message) => {
                    if self.adapter_message(message) {
                        break;
                    }
                }
                Inbound::EditorClosed => break,
                Inbound::AdapterClosed => {
                    if self.adapter_closed() {
                        break;
                    }
                }
            }
        }
        self.finish();
    }

    /// Handle one message from the editor; true when the session is over.
    fn editor_message(&mut self, message: Value) -> bool {
        if message["type"] == "response" {
            self.reply_to_reverse(message);
            return false;
        }
        if message["type"] != "request" {
            return false;
        }
        match message["command"].as_str().unwrap_or_default() {
            "initialize" => self.initialize(message),
            "launch" => self.launch(&message),
            "attach" => self.attach(&message),
            "disconnect" => return self.disconnect(message),
            "restart" => self.editor.respond(
                &message,
                Err(
                    "restarting isn't supported yet; stop the session and start it again, which \
                     rebuilds the app"
                        .into(),
                ),
            ),
            // The reader already cancelled the launch, if it was still going.
            "cancel" if message["arguments"]["progressId"] == LAUNCH_PROGRESS => {
                self.editor.respond(&message, Ok(None));
            }
            "configurationDone" => {
                self.forward(&message);
                if let Some(start) = self.start.take() {
                    let editor = Arc::clone(&self.editor);
                    // lldb-dap starts waiting for the app as it handles
                    // configurationDone; give it a moment to get there.
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(500));
                        if let Err(e) = start() {
                            editor.output("stderr", &error_text(&e));
                        }
                    });
                }
            }
            _ => self.forward(&message),
        }
        false
    }

    fn initialize(&mut self, message: Value) {
        let arguments = &message["arguments"];
        let flag = |key: &str, default: bool| arguments[key].as_bool().unwrap_or(default);
        let editor = &self.editor;
        editor
            .lines_from_one
            .store(flag("linesStartAt1", true), Ordering::Relaxed);
        editor
            .columns_from_one
            .store(flag("columnsStartAt1", true), Ordering::Relaxed);
        editor
            .progress
            .store(flag("supportsProgressReporting", false), Ordering::Relaxed);
        if self.adapter.is_none() {
            match Adapter::spawn(self.tx.clone(), self.log.clone()) {
                Ok(adapter) => self.adapter = Some(adapter),
                Err(e) => {
                    self.editor.respond(&message, Err(lldb_dap_missing(&e)));
                    return;
                }
            }
        }
        self.initializing = Some(message.clone());
        self.forward(&with_initialize_defaults(message));
    }

    fn launch(&mut self, message: &Value) {
        if let Err(why) = self.refuse_start() {
            self.editor.respond(message, Err(why));
            return;
        }
        if names_a_program(message) {
            self.pass_through(message, true);
            return;
        }
        self.editor.progress_start("Building");
        let prepared = LaunchConfig::from_arguments(&message["arguments"]).and_then(|config| {
            adapter::prepare_launch(self.ctx, &config, &*self.editor, &self.cancel)
        });
        self.editor.progress_end(match &prepared {
            Ok(_) => "Launched",
            Err(_) if self.cancel.is_cancelled() => "Cancelled",
            Err(_) => "Failed",
        });
        self.hand_over(message, true, prepared);
    }

    fn attach(&mut self, message: &Value) {
        if let Err(why) = self.refuse_start() {
            self.editor.respond(message, Err(why));
            return;
        }
        if names_a_program(message) {
            self.pass_through(message, false);
            return;
        }
        let prepared = LaunchConfig::from_arguments(&message["arguments"])
            .and_then(|config| adapter::prepare_attach(self.ctx, &config));
        self.hand_over(message, false, prepared);
    }

    /// Why a `launch` or `attach` can't start, checked before any build:
    /// lldb-dap isn't running (no `initialize` yet, or it failed), or the
    /// session already has an app.
    fn refuse_start(&self) -> Result<(), String> {
        if self.adapter.is_none() {
            return Err("lldb-dap isn't running; the session needs 'initialize' first".into());
        }
        if self.debuggee.is_some() {
            return Err("this session already has an app".into());
        }
        Ok(())
    }

    /// Forward a plain lldb-dap request untouched: there is nothing for the
    /// adapter to build.
    fn pass_through(&mut self, message: &Value, launched: bool) {
        let command = if launched { "launch" } else { "attach" };
        self.debuggee = Some(Debuggee {
            request_seq: message["seq"].clone(),
            editor_command: command.to_string(),
            forwarded: command,
            launched,
            app: None,
        });
        self.forward(message);
    }

    /// Send lldb-dap the request that reaches the prepared app, under the
    /// editor's `seq`, or answer the editor with why there is none.
    fn hand_over(&mut self, request: &Value, launched: bool, prepared: Result<Prepared, CliError>) {
        let prepared = match prepared {
            Ok(prepared) if self.adapter.is_some() => prepared,
            Ok(prepared) => {
                prepared.app.end(launched);
                self.editor
                    .respond(request, Err("lldb-dap isn't running".into()));
                return;
            }
            Err(e) => {
                let text = error_text(&e);
                self.editor.output("stderr", &text);
                self.editor.respond(request, Err(text));
                return;
            }
        };
        let forwarded = json!({
            "seq": request["seq"],
            "type": "request",
            "command": prepared.command,
            "arguments": prepared.arguments,
        });
        self.start = prepared.start;
        self.debuggee = Some(Debuggee {
            request_seq: request["seq"].clone(),
            editor_command: request["command"].as_str().unwrap_or_default().to_string(),
            forwarded: prepared.command,
            launched,
            app: Some(prepared.app),
        });
        self.forward(&forwarded);
    }

    /// Forward the editor's `disconnect`; true when the session is over.
    fn disconnect(&mut self, mut message: Value) -> bool {
        let asked = message["arguments"]["terminateDebuggee"].as_bool();
        let launched = self.debuggee.as_ref().is_some_and(|d| d.launched);
        let terminate = asked.unwrap_or(launched);
        // lldb-dap detaches from a process it attached to unless told
        // otherwise, and a launched app it attached to must still stop.
        if asked.is_none()
            && launched
            && self
                .debuggee
                .as_ref()
                .is_some_and(|d| d.forwarded == "attach")
        {
            if !message["arguments"].is_object() {
                message["arguments"] = json!({});
            }
            message["arguments"]["terminateDebuggee"] = json!(true);
        }
        if self.adapter.is_none() {
            self.editor.respond(&message, Ok(None));
            self.end_app(terminate);
            return true;
        }
        self.disconnecting = Some((message.clone(), terminate));
        self.forward(&message);
        false
    }

    /// Relay the editor's reply to a reverse request under lldb-dap's `seq`.
    fn reply_to_reverse(&mut self, mut message: Value) {
        if let Some(seq) = message["request_seq"]
            .as_i64()
            .and_then(|seq| self.reverse.remove(&seq))
        {
            message["request_seq"] = seq;
        }
        self.forward(&message);
    }

    fn forward(&mut self, message: &Value) {
        if let Some(adapter) = &mut self.adapter {
            adapter.send(message, &self.log);
        } else if message["type"] == "request" {
            self.editor
                .respond(message, Err("lldb-dap isn't running".into()));
        }
    }

    /// Relay one message from lldb-dap; true once it has answered the
    /// editor's `disconnect`, which ends the session.
    fn adapter_message(&mut self, mut message: Value) -> bool {
        match message["type"].as_str() {
            Some("response") => {
                let command = message["command"].as_str().unwrap_or_default().to_string();
                if command == "initialize" {
                    self.initializing = None;
                    // Restart isn't offered yet: editors then disconnect and
                    // launch again, which rebuilds.
                    if let Some(body) = message["body"].as_object_mut() {
                        body.remove("supportsRestartRequest");
                    }
                }
                if let Some(debuggee) = &self.debuggee
                    && message["request_seq"] == debuggee.request_seq
                    && command == debuggee.forwarded
                {
                    message["command"] = json!(debuggee.editor_command);
                }
                let disconnected = command == "disconnect" && self.disconnecting.is_some();
                self.editor.send(message);
                if disconnected && let Some((_, terminate)) = self.disconnecting.take() {
                    self.end_app(terminate);
                    return true;
                }
            }
            Some("request") => {
                let seq = message["seq"].clone();
                let ours = self.editor.send(message);
                self.reverse.insert(ours, seq);
            }
            _ => {
                if let Some(message) = without_launch_noise(message) {
                    self.editor.send(message);
                }
            }
        }
        false
    }

    /// lldb-dap exited; true when the session is over with it.
    fn adapter_closed(&mut self) -> bool {
        if let Some(adapter) = self.adapter.take() {
            adapter.stop();
        }
        if let Some(request) = self.initializing.take() {
            self.editor.respond(
                &request,
                Err(lldb_dap_missing(&CliError::new(
                    "lldb-dap exited before answering 'initialize'",
                ))),
            );
            return false;
        }
        if let Some((request, terminate)) = self.disconnecting.take() {
            self.editor.respond(&request, Ok(None));
            self.end_app(terminate);
            return true;
        }
        if self.debuggee.is_none() {
            return true;
        }
        // An exit nobody asked for: tell the editor the session is over, and
        // let it disconnect.
        self.editor.output("stderr", "lldb-dap exited unexpectedly");
        self.editor.event("terminated", &json!({}));
        false
    }

    fn end_app(&mut self, terminate: bool) {
        if let Some(app) = self.debuggee.as_mut().and_then(|d| d.app.take()) {
            app.end(terminate);
        }
    }

    fn finish(&mut self) {
        let terminate = self
            .disconnecting
            .as_ref()
            .map(|(_, terminate)| *terminate)
            .or_else(|| self.debuggee.as_ref().map(|d| d.launched))
            .unwrap_or(false);
        self.end_app(terminate);
        if let Some(adapter) = self.adapter.take() {
            adapter.stop();
        }
        self.editor.close();
    }
}

/// The editor's `initialize` with the fields the protocol lets it leave out
/// filled in with their defaults: Xcode's lldb-dap refuses one without
/// `pathFormat`.
fn with_initialize_defaults(mut message: Value) -> Value {
    if !message["arguments"].is_object() {
        message["arguments"] = json!({});
    }
    let arguments = &mut message["arguments"];
    for (key, default) in [("adapterID", "sweetpad"), ("pathFormat", "path")] {
        if !arguments[key].is_string() {
            arguments[key] = json!(default);
        }
    }
    message
}

/// An `output` event of the app's with the lines about its launch taken out
/// ([`adapter::is_launch_noise`]), or `None` when nothing else is left.
fn without_launch_noise(mut message: Value) -> Option<Value> {
    if message["event"] != "output"
        || !matches!(
            message["body"]["category"].as_str(),
            Some("stdout" | "stderr")
        )
    {
        return Some(message);
    }
    let text = message["body"]["output"].as_str()?;
    if !text.lines().any(adapter::is_launch_noise) {
        return Some(message);
    }
    let kept: String = text
        .split_inclusive('\n')
        .filter(|line| !adapter::is_launch_noise(line))
        .collect();
    if kept.trim().is_empty() {
        return None;
    }
    message["body"]["output"] = json!(kept);
    Some(message)
}

/// Whether a `launch` or `attach` names its `program`, as an lldb-dap
/// configuration does: a Swift package's binary, say. Such a request goes to
/// lldb-dap as it is, so an editor that sends every Swift session through
/// this adapter still debugs those.
fn names_a_program(request: &Value) -> bool {
    request["arguments"]["program"].is_string()
}

/// The text the editor shows for a failed launch: what was being done, what
/// went wrong, and the tip, on separate lines.
fn error_text(e: &CliError) -> String {
    let mut text = match e.headline() {
        Some(headline) => format!("{headline}: {}", e.detail()),
        None => e.detail().to_string(),
    };
    if let Some(tip) = e.tip_text() {
        text.push_str("\ntip: ");
        text.push_str(tip);
    }
    text
}

/// Why lldb-dap couldn't serve, and where to look.
fn lldb_dap_missing(e: &CliError) -> String {
    format!(
        "{}. 'sweetpad dap' needs Xcode's lldb-dap ('xcrun lldb-dap'); run 'sweetpad dap doctor' \
         to see what's missing",
        error_text(e)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_location_reads_line_and_column() {
        assert_eq!(
            parse_location("/src/App.swift:12:5"),
            Some(("/src/App.swift", 12, Some(5)))
        );
        assert_eq!(
            parse_location("/src/App.swift:12"),
            Some(("/src/App.swift", 12, None))
        );
        assert_eq!(parse_location("/src/App.swift"), None);
    }

    #[test]
    fn launch_noise_is_dropped_and_the_apps_lines_kept() {
        let event = |text: &str| json!({ "type": "event", "event": "output", "body": { "category": "stdout", "output": text } });
        let stub =
            "2026-10-04 22:52:10.02+0200 App[1:2] [PreviewsAgentExecutorLibrary] Looking up\r\n";
        assert!(without_launch_noise(event(stub)).is_none());
        let mixed = format!("{stub}hello\r\n");
        let kept = without_launch_noise(event(&mixed)).unwrap();
        assert_eq!(kept["body"]["output"], "hello\r\n");
        let console = json!({ "type": "event", "event": "output", "body": { "category": "console", "output": stub } });
        assert!(without_launch_noise(console).is_some());
    }

    #[test]
    fn initialize_gets_the_fields_lldb_dap_requires() {
        let filled = with_initialize_defaults(json!({ "seq": 1, "command": "initialize" }));
        assert_eq!(filled["arguments"]["pathFormat"], "path");
        assert_eq!(filled["arguments"]["adapterID"], "sweetpad");
        let kept = with_initialize_defaults(
            json!({ "arguments": { "pathFormat": "uri", "adapterID": "nvim" } }),
        );
        assert_eq!(kept["arguments"]["pathFormat"], "uri");
        assert_eq!(kept["arguments"]["adapterID"], "nvim");
    }

    #[test]
    fn ending_requests_cancel_the_launch() {
        let request = |command: &str, args: Value| json!({ "type": "request", "command": command, "arguments": args });
        assert!(ends_launch(&request("disconnect", json!({}))));
        assert!(ends_launch(&request("terminate", json!({}))));
        assert!(ends_launch(&request(
            "cancel",
            json!({ "progressId": LAUNCH_PROGRESS })
        )));
        assert!(!ends_launch(&request("cancel", json!({ "requestId": 3 }))));
        assert!(!ends_launch(&request("threads", json!({}))));
    }
}
