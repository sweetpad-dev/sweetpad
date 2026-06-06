// Self-contained Build Server Protocol server. sourcekit-lsp execs this file
// (via the `argv` in buildServer.json) and speaks BSP over its stdio; we hand
// that straight to the sweetpad-lib BSP server in the native addon. It loads the
// addon directly — no running extension required — so it works whenever
// sourcekit-lsp decides to spawn it.
//
// Shipped as `out/bsp-server.js` next to `out/lib/` (the copied addon); the
// `@sweetpad/lib` import is rewritten to `./lib/index.js` at bundle time.
import { bsp } from "@sweetpad/lib";

// `bsp` blocks, running the JSON-RPC loop over stdin/stdout until EOF /
// `build/exit`. Args are the server flags (`--project`, `--xcode`,
// `--derived-data-path`) written into buildServer.json by the extension.
bsp(process.argv.slice(2));
