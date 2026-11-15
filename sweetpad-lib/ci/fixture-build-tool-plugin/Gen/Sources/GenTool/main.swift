import Foundation

let output = CommandLine.arguments[1]
try "let generatedGreeting = \"hello from the plugin\"\n"
    .write(toFile: output, atomically: true, encoding: .utf8)
