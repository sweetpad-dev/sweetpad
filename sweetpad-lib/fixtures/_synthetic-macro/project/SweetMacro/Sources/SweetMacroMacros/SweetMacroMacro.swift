import SwiftCompilerPlugin
import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

/// Implementation of the `stringify` macro: takes an expression of any type and
/// produces a tuple containing the value and the source code that produced it.
/// `#stringify(x + y)` expands to `(x + y, "x + y")`.
public struct StringifyMacro: ExpressionMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) -> ExprSyntax {
        guard let argument = node.arguments.first?.expression else {
            fatalError("compiler bug: the macro does not have any arguments")
        }
        return "(\(argument), \(literal: argument.description))"
    }
}

@main
struct SweetMacroPlugin: CompilerPlugin {
    let providingMacros: [Macro.Type] = [StringifyMacro.self]
}
