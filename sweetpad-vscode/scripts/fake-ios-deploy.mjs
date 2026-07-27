#!/usr/bin/env node
/**
 * Stand-in for ios-deploy, for exercising the iOS 16-and-below debug route without an old
 * device. It replays the console output a real ios-deploy prints on the way to its debug
 * phase, opens a TCP listener on the port it announces, and stays up until killed.
 *
 * That covers everything on this side of the wire — argument construction, output parsing,
 * the workspace-state write, watch-marker ordering, and the debug configuration CodeLLDB
 * resolves. The one thing it cannot fake is LLDB's gdb-remote handshake, so the debug session
 * fails at "gdb-remote 127.0.0.1:<port>". Reaching that failure means the extension did its
 * whole job; only the device leg is left, and that needs real hardware.
 *
 * Usage — pair it with scripts/fake-xcrun.mjs, which supplies the old device this method
 * needs, and note that prefixing PATH before launching VS Code is NOT enough: tools are
 * spawned with the environment resolved from the user's login shell, which overrides the
 * extension host's own PATH. Point "sweetpad.shellEnv.shell" at a wrapper instead.
 *
 *   BIN=/tmp/sweetpad-fakes && mkdir -p $BIN
 *   ln -sf "$PWD/scripts/fake-ios-deploy.mjs" $BIN/ios-deploy
 *   ln -sf "$PWD/scripts/fake-xcrun.mjs" $BIN/xcrun
 *   printf '#!/bin/sh\nexport PATH="%s:$PATH"\nfor last; do :; done\nexec /bin/sh -c "$last"\n' "$BIN" > $BIN/shell
 *   chmod +x $BIN/shell
 *
 * Then in the target workspace's .vscode/settings.json:
 *
 *   { "sweetpad.shellEnv.shell": "/tmp/sweetpad-fakes/shell" }
 *
 * Pick the "Fake iPad mini 4" destination and start the "SweetPad: Build and Run (Wait for
 * debugger)" configuration. The app bundle at the resolved appPath has to exist and hold a
 * real Mach-O — LLDB rejects a placeholder at "target create", before it ever dials the port.
 *
 * Environment knobs:
 *   SWEETPAD_FAKE_DELAY_MS  pause between phases, for watching marker ordering (default 300)
 *   SWEETPAD_FAKE_FAIL=1    die after install, the way a first-connection flake does
 *   SWEETPAD_FAKE_HANG=1    never reach the debug phase, to check the timeout
 */

import net from "node:net";
import path from "node:path";

const args = process.argv.slice(2);

function argValue(name) {
  const index = args.indexOf(name);
  return index === -1 ? undefined : args[index + 1];
}

const bundle = argValue("--bundle") ?? "/path/to/Unknown.app";
const deviceId = argValue("--id") ?? "00008110-001234567890001E";
const appName = path.basename(bundle.replace(/\/$/, ""));
const delayMs = Number.parseInt(process.env.SWEETPAD_FAKE_DELAY_MS ?? "300", 10);

function say(line) {
  process.stdout.write(`${line}\r\n`);
}

function pause() {
  return new Promise((resolve) => setTimeout(resolve, delayMs));
}

if (args.includes("--version")) {
  say("1.12.2");
  process.exit(0);
}

if (!args.includes("--nolldb")) {
  say("fake-ios-deploy: expected --nolldb (the debug route must not let ios-deploy own LLDB)");
  process.exit(1);
}

say(`[  0%] Found ${deviceId} connected through USB, beginning install`);
await pause();
say("[ 95%] GeneratingApplicationMap");
say(`[100%] Installed package ${bundle}`);

if (process.env.SWEETPAD_FAKE_FAIL === "1") {
  // A real first-connection flake dies here: after install, before the debug phase.
  say("[  0%] Looking up developer disk image");
  say("error: could not start device support");
  process.exit(253);
}

say("------ Debug phase ------");
say(
  `Starting debug of ${deviceId} (J96AP, iPad mini 4, iphoneos, arm64, 15.6.1, 19G82) a.k.a. 'iPad mini' connected through USB...`,
);
say("[  0%] Looking up developer disk image");
await pause();

if (process.env.SWEETPAD_FAKE_HANG === "1") {
  setInterval(() => say("[ 50%] Still looking up developer disk image"), 5000);
} else {
  say("[ 95%] Developer disk image mounted successfully");
  say(`Symbol Path: ${process.env.HOME}/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols`);

  // Bind before announcing, so a debugger connecting the instant it reads the port finds a
  // listener rather than a closed one. LLDB still fails the handshake — see the note above.
  const server = net.createServer((socket) => {
    process.stderr.write("fake-ios-deploy: debugger connected (handshake will fail, this is expected)\r\n");
    socket.on("error", () => {});
  });

  server.listen(0, "127.0.0.1", () => {
    const address = server.address();
    const port = typeof address === "object" && address ? address.port : 0;

    say("[100%] Listening for lldb connections");
    say("-------------------------");
    say(`debugserver port: ${port}`);
    say(`App path: /private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/${appName}`);
  });

  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.on(signal, () => {
      server.close();
      process.exit(0);
    });
  }
}
