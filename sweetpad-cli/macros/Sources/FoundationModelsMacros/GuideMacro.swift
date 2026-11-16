import OpenAppleMacrosBase
import SwiftDiagnostics

struct GuideMacro: PeerMacro, AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        validate(node, declaration: declaration, in: context)
        return []
    }

    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        return []
    }

    private static func validate(
        _ node: AttributeSyntax,
        declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) {
        guard let variable = declaration.as(VariableDeclSyntax.self),
              !variable.modifiers.contains(where: {
                  $0.name.tokenKind == .keyword(.static) || $0.name.tokenKind == .keyword(.class)
              }),
              variable.bindings.allSatisfy({ $0.accessorBlock == nil }) else {
            context.diagnose(Diagnostic(
                node: node,
                message: FoundationModelsDiagnostic("'@Guide' can only be used with a stored property.")
            ))
            return
        }
    }
}

struct FoundationModelsDiagnostic: DiagnosticMessage, FixItMessage {
    let message: String
    let severity: DiagnosticSeverity

    init(_ message: String, severity: DiagnosticSeverity = .error) {
        self.message = message
        self.severity = severity
    }

    var diagnosticID: MessageID {
        MessageID(domain: "FoundationModelsMacros", id: message)
    }
    var fixItID: MessageID { diagnosticID }
}
