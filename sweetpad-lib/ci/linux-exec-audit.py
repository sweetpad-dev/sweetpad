#!/usr/bin/env python3
"""Run a Linux command under strace and reject macOS executable use.

This is a provenance check, not a security sandbox. It does not change binfmt
registrations. A successful build cannot pass the audit if it attempted to run
a Mach-O tool, or if a successfully executed tool cannot be inspected.
"""
import argparse
import ast
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

MACHO = {bytes.fromhex(s) for s in ["feedface", "cefaedfe", "feedfacf", "cffaedfe", "cafebabe", "bebafeca", "cafebabf", "bfbafeca"]}
QUOTED = r'"(?:\\.|[^"\\])*"'


def string(token):
    return os.fsdecode(ast.literal_eval(token).encode("latin1"))


def events(text):
    pending = {}
    result = []
    for number, line in enumerate(text.splitlines()):
        match = re.match(r"(\d+)\s+(.*)", line)
        if not match:
            continue
        pid, body = int(match[1]), match[2]
        if "<unfinished ...>" in body:
            pending[pid] = (number, body.replace("<unfinished ...>", ""))
            continue
        resumed = re.match(r"<\.\.\. \w+ resumed>(.*)", body)
        if resumed and pid in pending:
            number, prefix = pending.pop(pid)
            body = prefix + resumed[1]
        result.append((number, pid, body))
    # An interrupted exec attempt must still be inspected.
    result.extend((number, pid, body) for pid, (number, body) in pending.items())
    return sorted(result)


def inspect_trace(text, initial_cwd, snapshots=None):
    rows = events(text)
    parents, groups = {}, {}

    def group(pid):
        while pid in groups:
            pid = groups[pid]
        return pid

    for number, pid, body in rows:
        match = re.match(r"(?:clone3?|fork|vfork)\(.*\)\s*=\s*(\d+)\s*$", body)
        if match:
            child = int(match[1])
            if "CLONE_FS" in body:
                groups[child] = group(pid)
            else:
                parents[child] = (pid, number)
    histories = {}
    for number, pid, body in rows:
        if not re.search(r"=\s*0\s*$", body):
            continue
        match = re.match(r"chdir\((" + QUOTED + r")\)", body)
        fd = re.match(r"fchdir\(\d+<([^>]+)>\)", body)
        if match or fd:
            path = string(match[1]) if match else fd[1]
            histories.setdefault(group(pid), []).append((number, path))

    def cwd(pid, before, seen=None):
        pid = group(pid)
        seen = set() if seen is None else seen
        marker = (pid, before)
        if marker in seen:
            raise ValueError("cyclic process working-directory history")
        seen.add(marker)
        history = [item for item in histories.get(pid, []) if item[0] < before]
        if history:
            number, path = history[-1]
            return Path(path) if Path(path).is_absolute() else cwd(pid, number, seen) / path
        if pid in parents:
            parent, number = parents[pid]
            return cwd(parent, number, seen)
        return Path(initial_cwd)

    tools, violations = {}, []
    for number, pid, body in rows:
        match = re.match(r"execve\((" + QUOTED + r")", body)
        at = re.match(r"execveat\(([^,]+),\s*(" + QUOTED + r")", body)
        if not match and not at:
            continue
        name = string(match[1] if match else at[2])
        base = cwd(pid, number)
        if at and not at[1].startswith("AT_FDCWD"):
            fd = re.search(r"<([^>]+)>", at[1])
            if not fd:
                violations.append({"pid": pid, "path": name, "reason": "unresolved execveat descriptor"})
                continue
            base = Path(fd[1])
        path = Path(name) if name else base
        if not path.is_absolute():
            path = base / path
        try:
            captured = (snapshots or {}).get((pid, name))
            if captured:
                resolved, header = captured
            else:
                resolved = path.resolve(strict=True)
                with resolved.open("rb") as tool:
                    header = tool.read(512)
        except OSError:
            if re.search(r"= -1 (?:ENOENT|ENOTDIR|EACCES)", body):
                continue
            violations.append({"pid": pid, "path": str(path), "reason": "executed tool is unavailable for inspection"})
            continue
        kind = "Mach-O" if header[:4] in MACHO else "ELF" if header.startswith(b"\x7fELF") else "script" if header.startswith(b"#!") else "other"
        tools[str(resolved)] = {"format": kind, "attempts": tools.get(str(resolved), {}).get("attempts", 0) + 1}
        if kind == "Mach-O":
            violations.append({"pid": pid, "path": str(resolved), "reason": "macOS executable attempted"})
        elif kind == "script":
            interpreter = os.fsdecode(header.splitlines()[0][2:]).strip().split()[0]
            try:
                with Path(interpreter).open("rb") as tool:
                    if tool.read(4) in MACHO:
                        violations.append({"pid": pid, "path": interpreter, "reason": "macOS shebang interpreter"})
            except OSError:
                violations.append({"pid": pid, "path": interpreter, "reason": "unavailable shebang interpreter"})
        elif kind == "other" and not re.search(r"= -1", body):
            violations.append({"pid": pid, "path": str(resolved), "reason": "unrecognized executable format"})
    return {"tools": tools, "violations": violations, "passed": bool(tools) and not violations}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cwd", type=Path, default=Path.cwd())
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if sys.platform != "linux" or not shutil.which("strace") or not command:
        parser.error("needs Linux, strace, and a command after --")
    args.out.mkdir(parents=True, exist_ok=False)
    trace = args.out / "exec.log"
    # Inspect while tracees exist: SwiftPM deletes temporary manifest binaries
    # after running them. A short exit delay keeps even tiny tools inspectable;
    # missing captures still fail closed in inspect_trace.
    read_fd, write_fd = os.pipe()
    snapshots = {}
    process = subprocess.Popen(["strace", "-f", "-qq", "-yy", "-s", "4096", "-e",
        "trace=execve,execveat,chdir,fchdir,clone,clone3,fork,vfork,exit_group",
        "-e", "inject=exit_group:delay_enter=100ms", "-o", f"/dev/fd/{write_fd}",
        "--", *command], cwd=args.cwd, pass_fds=(write_fd,))
    os.close(write_fd)
    with os.fdopen(read_fd) as stream, trace.open("w") as log:
        for line in stream:
            log.write(line)
            match = re.match(r"(\d+)\s+execve\((" + QUOTED + r")", line)
            at = re.match(r"(\d+)\s+execveat\(([^,]+),\s*(" + QUOTED + r")", line)
            if not match and not at:
                continue
            pid = int((match or at)[1])
            name = string(match[2] if match else at[3])
            base = Path(f"/proc/{pid}/cwd")
            if at and not at[2].startswith("AT_FDCWD"):
                descriptor = re.search(r"<([^>]+)>", at[2])
                if not descriptor: continue
                base = Path(descriptor[1])
            path = Path(name) if name else base
            if not path.is_absolute(): path = base / path
            try:
                resolved = path.resolve(strict=True)
                with resolved.open("rb") as tool: header = tool.read(512)
                snapshots[(pid,name)] = (resolved,header)
            except OSError:
                # The post-run parser accounts for failed PATH lookups and
                # rejects successful execs without a readable capture.
                pass
    status = process.wait()
    (args.out / "headers.json").write_text(json.dumps([
        {"pid":pid,"requested":name,"resolved":str(path),"header":header.hex()}
        for (pid,name),(path,header) in snapshots.items()],indent=2)+"\n")
    report = inspect_trace(trace.read_text(), args.cwd.resolve(), snapshots)
    report.update(command=command, exitCode=status)
    report["passed"] &= status == 0
    (args.out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    if report["violations"]:
        print(json.dumps(report["violations"], indent=2), file=sys.stderr)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
