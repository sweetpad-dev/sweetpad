import OpenAppleMacrosBase
import SwiftDiagnostics

struct EntryMacro: PeerMacro, AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let ext = context.lexicalContext.first?.as(ExtensionDeclSyntax.self)

        let kind: EntryKind
        switch ext.map({ "\($0.extendedType.trimmed)" }) {
        case "EnvironmentValues", "SwiftUI.EnvironmentValues", "SwiftUICore.EnvironmentValues":
            kind = .environmentValues
        case "Transaction", "SwiftUI.Transaction", "SwiftUICore.Transaction":
            kind = .transaction
        case "ContainerValues", "SwiftUI.ContainerValues", "SwiftUICore.ContainerValues":
            kind = .containerValues
        case "FocusedValues", "SwiftUI.FocusedValues":
            kind = .focusedValues
        default:
            context.diagnose(Diagnostic(
                node: node,
                message: EntryDiagnostic("'@Entry' macro can only attach to var declarations inside extensions of EnvironmentValues, ContainerValues, Transaction, or FocusedValues"),
                highlights: [Syntax(declaration)]
            ))
            return []
        }

        guard let varDecl = declaration.as(VariableDeclSyntax.self) else {
            return []
        }
        if varDecl.bindingSpecifier.tokenKind == .keyword(.let) {
            diagnoseLet(node: node, declaration: varDecl, in: context)
            return []
        }
        guard varDecl.bindings.count == 1,
              let binding = varDecl.bindings.first,
              let pattern = binding.pattern.as(IdentifierPatternSyntax.self) else {
            context.diagnose(Diagnostic(
                node: node,
                message: EntryDiagnostic("'@Entry' can only be applied to a 'var' declaration with a simple name"),
                highlights: [Syntax(varDecl)]
            ))
            return []
        }

        if kind == .focusedValues {
            guard let optionalType = binding.typeAnnotation?.type.as(OptionalTypeSyntax.self) else {
                throw MacroError("'@Entry' on 'FocusedValues' requires an optional type")
            }
            return [
                """
                private struct __Key_\(pattern.identifier.trimmed): \(kind.keyType) {
                    typealias Value = \(optionalType.wrappedType.trimmed)
                }
                """
            ]
        }

        let initializer: ExprSyntax
        if let value = binding.initializer?.value {
            initializer = value
        } else if binding.typeAnnotation?.type.is(OptionalTypeSyntax.self) == true {
            initializer = "nil"
        } else {
            let position = binding.endPositionBeforeTrailingTrivia
            context.diagnose(Diagnostic(
                node: pattern,
                message: EntryDiagnostic("Property missing a default value"),
                highlights: [Syntax(binding)],
                fixIts: [FixIt(
                    message: EntryDiagnostic("Provide default value"),
                    changes: [.replaceText(range: position..<position, with: " = <#default value#>", in: Syntax(binding))]
                )]
            ))
            return []
        }

        let typeAnnotation = binding.typeAnnotation.map { ": \($0.type.trimmed)" } ?? ""
        return [
            """
            private struct __Key_\(pattern.identifier.trimmed): \(kind.keyType) {
                @SwiftUICore.__EntryDefaultValue
                static var defaultValue\(raw: typeAnnotation) = \(initializer.trimmed)
            }
            """
        ]
    }

    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let varDecl = declaration.as(VariableDeclSyntax.self),
              varDecl.bindings.count == 1,
              let binding = varDecl.bindings.first,
              let pattern = binding.pattern.as(IdentifierPatternSyntax.self) else {
            return []
        }
        if varDecl.bindingSpecifier.tokenKind == .keyword(.let) {
            diagnoseLet(node: node, declaration: varDecl, in: context)
            return []
        }
        return [
            "get { self[__Key_\(pattern.identifier.trimmed).self] }",
            "set { self[__Key_\(pattern.identifier.trimmed).self] = newValue }",
            "_modify { yield &self[__Key_\(pattern.identifier.trimmed).self] }",
        ]
    }
}

private func diagnoseLet(
    node: AttributeSyntax,
    declaration: VariableDeclSyntax,
    in context: some MacroExpansionContext
) {
    let token = declaration.bindingSpecifier
    context.diagnose(Diagnostic(
        node: node,
        message: EntryDiagnostic("'@Entry' can only be applied to a 'var' declaration"),
        highlights: [Syntax(declaration)],
        fixIts: [FixIt(
            message: EntryDiagnostic("Replace 'let' with 'var'"),
            changes: [.replace(
                oldNode: Syntax(token),
                newNode: Syntax(TokenSyntax.keyword(
                    .var,
                    leadingTrivia: token.leadingTrivia,
                    trailingTrivia: token.trailingTrivia
                ))
            )]
        )]
    ))
}

private struct EntryDiagnostic: DiagnosticMessage, FixItMessage {
    let message: String

    init(_ message: String) {
        self.message = message
    }

    var diagnosticID: MessageID { MessageID(domain: "SwiftUIMacros", id: message) }
    var fixItID: MessageID { diagnosticID }
    var severity: DiagnosticSeverity { .error }
}

struct EntryDefaultValueMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let varDecl = declaration.as(VariableDeclSyntax.self),
              varDecl.bindings.count == 1,
              let initializer = varDecl.bindings.first?.initializer else {
            throw MacroError("'@__EntryDefaultValue' requires an initialized variable")
        }
        return ["get { \(initializer.value.trimmed) }"]
    }
}

private enum EntryKind {
    case environmentValues
    case transaction
    case containerValues
    case focusedValues

    var keyType: TypeSyntax {
        switch self {
        case .environmentValues:
            return "SwiftUICore.EnvironmentKey"
        case .transaction:
            return "SwiftUICore.TransactionKey"
        case .containerValues:
            return "SwiftUICore.ContainerValueKey"
        case .focusedValues:
            return "SwiftUI.FocusedValueKey"
        }
    }
}
