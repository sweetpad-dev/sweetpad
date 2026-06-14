// The Build Server Protocol server sourcekit-lsp launches (via `argv` in
// buildServer.json). Bundled to `out/bsp-server.js` with a `#!/usr/bin/env node`
// shebang — like the CLI — so sourcekit-lsp execs it directly through system
// Node, which loads the native addon and runs the BSP loop over stdio. The
// extension passes `--config <bsp.json>` in argv; absent that, the server
// discovers the config from its cwd via the host-wide project index.
import { bsp } from "@sweetpad/lib";

bsp(process.argv.slice(2));
