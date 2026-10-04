#!/usr/bin/env python3
"""One real debug session through `sweetpad dap`, the way an editor drives it.

Starts `sweetpad dap`, launches the scheme on the destination, stops at a
breakpoint, reads the stack there, and disconnects, checking along the way
that the app's own output reached the debug console and that every message
the adapter sent carries a larger `seq` than the one before it.

    dap-session.py SWEETPAD_BIN --cwd DIR --scheme NAME --destination REF \
        --breakpoint FILE:LINE [--expect-output TEXT] [--attach]

Exits 0 when the session went as an editor expects, 1 otherwise. Run by
ci/smoke.sh.
"""

import argparse
import json
import queue
import subprocess
import sys
import threading
import time


class Session:
    def __init__(self, argv, cwd):
        self.proc = subprocess.Popen(
            argv, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        self.messages = queue.Queue()
        self.seq = 0
        self.last_seq = 0
        self.output = []
        self.stderr = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._drain_stderr, daemon=True).start()

    def _read(self):
        stream = self.proc.stdout
        while True:
            headers = {}
            while True:
                line = stream.readline()
                if not line:
                    self.messages.put(None)
                    return
                line = line.decode().strip()
                if not line:
                    break
                key, value = line.split(":", 1)
                headers[key.strip().lower()] = value.strip()
            body = stream.read(int(headers["content-length"]))
            self.messages.put(json.loads(body))

    def _drain_stderr(self):
        for line in self.proc.stderr:
            self.stderr.append(line.decode(errors="replace").rstrip())

    def send(self, command, arguments=None):
        self.seq += 1
        message = {"seq": self.seq, "type": "request", "command": command}
        if arguments is not None:
            message["arguments"] = arguments
        body = json.dumps(message).encode()
        self.proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        self.proc.stdin.flush()
        return self.seq

    def wait(self, predicate, timeout, what):
        deadline = time.time() + timeout
        while time.time() < deadline:
            message = self.next(0.5, what)
            if message is not None and predicate(message):
                return message
        fail(f"timed out waiting for {what}", self)

    def next(self, timeout, what):
        """The adapter's next message within `timeout`, recorded, or None."""
        try:
            message = self.messages.get(timeout=timeout)
        except queue.Empty:
            return None
        if message is None:
            fail(f"the adapter exited while waiting for {what}", self)
        seq = message.get("seq", 0)
        if seq <= self.last_seq:
            fail(f"seq went from {self.last_seq} to {seq} on {json.dumps(message)[:200]}", self)
        self.last_seq = seq
        if message.get("event") == "output":
            text = message["body"].get("output", "")
            self.output.append(text)
            print("  " + text.rstrip(), flush=True)
        return message

    def response(self, seq, command, timeout):
        message = self.wait(
            lambda m: m.get("type") == "response" and m.get("request_seq") == seq,
            timeout,
            f"the {command} response",
        )
        if message.get("command") != command:
            fail(f"the {command} response came back as {message.get('command')!r}", self)
        if not message.get("success"):
            fail(f"{command} failed: {message.get('message')}", self)
        return message

    def event(self, name, timeout):
        return self.wait(
            lambda m: m.get("type") == "event" and m.get("event") == name, timeout, f"the {name} event"
        )


def fail(why, session=None):
    print(f"FAIL: {why}", flush=True)
    if session is not None:
        for line in session.stderr[-20:]:
            print(f"  stderr: {line}")
        session.proc.kill()
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("sweetpad")
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--scheme", required=True)
    parser.add_argument("--destination", required=True)
    parser.add_argument("--breakpoint", required=True, help="FILE:LINE")
    parser.add_argument("--expect-output", help="text the app prints before the breakpoint")
    parser.add_argument("--xcodebuild-arg", action="append", default=[], help="repeatable")
    parser.add_argument("--build-timeout", type=int, default=900)
    args = parser.parse_args()
    path, line = args.breakpoint.rsplit(":", 1)

    session = Session([args.sweetpad, "dap"], args.cwd)
    seq = session.send(
        "initialize",
        {
            "clientID": "dap-session",
            "adapterID": "sweetpad",
            "linesStartAt1": True,
            "columnsStartAt1": True,
            "pathFormat": "path",
            "supportsProgressReporting": True,
        },
    )
    capabilities = session.response(seq, "initialize", 30).get("body") or {}
    if capabilities.get("supportsRestartRequest"):
        fail("the adapter offers restart, which it doesn't handle yet", session)
    print("initialized", flush=True)

    seq = session.send(
        "launch",
        {
            "type": "sweetpad",
            "request": "launch",
            "cwd": args.cwd,
            "scheme": args.scheme,
            "destination": args.destination,
            "xcodebuildArgs": args.xcodebuild_arg,
        },
    )
    session.response(seq, "launch", args.build_timeout)
    print("launched", flush=True)
    session.event("initialized", 60)

    seq = session.send(
        "setBreakpoints", {"source": {"path": path}, "breakpoints": [{"line": int(line)}]}
    )
    session.response(seq, "setBreakpoints", 60)
    seq = session.send("configurationDone", {})
    session.response(seq, "configurationDone", 60)

    stopped = session.event("stopped", 120)
    if stopped["body"].get("reason") != "breakpoint":
        fail(f"stopped for {stopped['body'].get('reason')!r}, not the breakpoint", session)
    thread = stopped["body"]["threadId"]
    seq = session.send("stackTrace", {"threadId": thread, "levels": 1})
    frames = session.response(seq, "stackTrace", 30)["body"]["stackFrames"]
    top = frames[0] if frames else {}
    where = f"{(top.get('source') or {}).get('path')}:{top.get('line')}"
    if not where.endswith(f"{path.rsplit('/', 1)[-1]}:{line}"):
        fail(f"stopped at {where}, not {args.breakpoint}", session)
    print(f"stopped at {where}", flush=True)

    if args.expect_output:
        deadline = time.time() + 10
        while not any(args.expect_output in text for text in session.output):
            if time.time() > deadline:
                fail(f"the app's {args.expect_output!r} never reached the console", session)
            session.next(0.5, "app output")

    seq = session.send("disconnect", {})
    session.response(seq, "disconnect", 60)
    try:
        session.proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        fail("the adapter kept running after disconnect", session)
    print("PASS", flush=True)


if __name__ == "__main__":
    main()
