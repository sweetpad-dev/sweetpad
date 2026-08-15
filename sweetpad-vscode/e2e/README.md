# End-to-end suite

These tests run inside a real VS Code extension host (see `../.vscode-test.mjs`) against a committed Xcode fixture.
`vscode` here is the genuine API, not the unit tests' stub, so this layer covers what that stub cannot: activation,
command registration, settings resolution, and what actually lands on disk.

## The rule: assert outcomes, never mechanism

The build pipeline is expected to move from invoking `xcodebuild` directly to calling into sweetpad-core. **This suite
has to survive that swap unchanged** — that is what makes it useful as the safety net for the migration rather than a
second thing to rewrite.

So a test may assert only what a user could observe:

- a **command** exists, runs, and completes
- a **setting** is honoured — pointing DerivedData somewhere puts products there
- a **file** appears: a build product, a `buildServer.json` SourceKit-LSP accepts
- a **diagnostic** shows up in the Problems panel at the right file, line and severity, and clears when the code is
  fixed
- a **task** starts and reports an exit status

And a test may never assert:

- the text of a terminal, or that the word `xcodebuild` appears anywhere
- the exact argument vector handed to any tool
- the internal layout of DerivedData beyond "the product is discoverable under the directory we chose" — locate products
  by searching, not by hardcoding `Build/Products/<configuration>/`
- log file paths, log contents, or which binary was executed

If a test fails after the pipeline is swapped, that should mean the behaviour changed — not that the implementation did.

## Running

    npm run test:e2e          # builds the extension, compiles the suite, runs it

One VS Code instance at a time: the harness cannot run concurrently with another `vscode-test` run, and killing one
mid-flight can corrupt `.vscode-test/extensions` (delete that directory if the extension stops activating).
