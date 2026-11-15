import Foundation
import PackagePlugin

@main
struct GenPlugin: BuildToolPlugin {
    func createBuildCommands(context: PluginContext, target: Target) throws -> [Command] {
        try generate(into: context.pluginWorkDirectoryURL, with: context.tool(named: "GenTool").url)
    }
}

/// One command that writes `Generated.swift` into `folder`.
func generate(into folder: URL, with tool: URL) -> [Command] {
    let output = folder.appending(path: "Generated.swift")
    return [
        .buildCommand(
            displayName: "Generate greeting",
            executable: tool,
            arguments: [output.path],
            outputFiles: [output]
        )
    ]
}

#if canImport(XcodeProjectPlugin)
import XcodeProjectPlugin

extension GenPlugin: XcodeBuildToolPlugin {
    func createBuildCommands(context: XcodePluginContext, target: XcodeTarget) throws -> [Command] {
        try generate(into: context.pluginWorkDirectoryURL, with: context.tool(named: "GenTool").url)
    }
}
#endif
