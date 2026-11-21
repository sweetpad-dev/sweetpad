import Foundation
import PackagePlugin

@main
struct GenPlugin: BuildToolPlugin {
    func createBuildCommands(context: PluginContext, target: Target) throws -> [Command] {
        guard let module = target as? SourceModuleTarget else { return [] }
        return try generate(
            inputs: module.sourceFiles(withSuffix: ".swift").map(\.url),
            into: context.pluginWorkDirectoryURL, with: context.tool(named: "GenTool").url)
    }
}

func generate(inputs: [URL], into folder: URL, with tool: URL) -> [Command] {
    let source = folder.appending(path: "Generated.swift")
    let resource = folder.appending(path: "PluginInputs.txt")
    return [
        .buildCommand(
            displayName: "Generate greeting",
            executable: tool,
            arguments: [source.path, resource.path] + inputs.map(\.path),
            inputFiles: inputs,
            outputFiles: [source, resource]
        )
    ]
}

#if canImport(XcodeProjectPlugin)
import XcodeProjectPlugin

extension GenPlugin: XcodeBuildToolPlugin {
    func createBuildCommands(context: XcodePluginContext, target: XcodeTarget) throws -> [Command] {
        try generate(
            inputs: target.inputFiles.filter { $0.url.pathExtension == "swift" }.map(\.url),
            into: context.pluginWorkDirectoryURL, with: context.tool(named: "GenTool").url)
    }
}
#endif
