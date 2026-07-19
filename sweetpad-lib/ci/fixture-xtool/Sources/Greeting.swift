/// A second source file so the generated package exercises the xtool backend's
/// multi-file source-graph reconstruction, not just a single `@main` file.
enum Greeting {
    static let message = "Hello from a Linux cross-build"
}
