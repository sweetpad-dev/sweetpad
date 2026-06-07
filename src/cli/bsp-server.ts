// The Build Server Protocol server sourcekit-lsp launches (via `argv` in
// buildServer.json). Bundled to `out/bsp-server.js` with a `#!/usr/bin/env node`
// shebang — like the CLI — so sourcekit-lsp execs it directly through system
// Node, which loads the native addon and runs the BSP loop over stdio. No
// arguments: the server discovers the project/config over the control socket.
import { bsp } from "@sweetpad/lib";

bsp(process.argv.slice(2));
