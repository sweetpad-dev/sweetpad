# SweetPad

Develop Swift/iOS projects from VS Code and the terminal

- [`sweetpad-vscode/`](./sweetpad-vscode) — VS Code extension ([Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad)); the N-API addon bridging it to the Rust core lives in [`sweetpad-vscode/native/`](./sweetpad-vscode/native)
- [`sweetpad-cli/`](./sweetpad-cli) — the standalone `sweetpad` CLI
- [`sweetpad-core/`](./sweetpad-core) — business logic shared by the CLI and the extension (build-settings resolution, BSP server)
- [`sweetpad-lib/`](./sweetpad-lib) — interface-agnostic Xcode file/format utilities and the build-settings resolver
- [`sweetpad-docs/`](./sweetpad-docs) — documentation site
