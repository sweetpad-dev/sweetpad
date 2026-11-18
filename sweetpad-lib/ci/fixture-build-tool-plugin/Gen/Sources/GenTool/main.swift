import Foundation

let output = CommandLine.arguments[1]
try "let generatedGreeting = \"hello from the plugin\"\n"
    .write(toFile: output, atomically: true, encoding: .utf8)

let inputs = CommandLine.arguments.dropFirst(3).map { URL(fileURLWithPath: $0).lastPathComponent }
try inputs.sorted().joined(separator: "\n")
    .write(toFile: CommandLine.arguments[2], atomically: true, encoding: .utf8)
