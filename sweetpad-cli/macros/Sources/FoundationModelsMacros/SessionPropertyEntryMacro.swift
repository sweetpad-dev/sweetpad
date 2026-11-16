import OpenAppleMacrosBase
import SwiftDiagnostics

struct SessionPropertyEntryMacro: PeerMacro, AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let extensionType = context.lexicalContext.first?.as(ExtensionDeclSyntax.self)?.extendedType.trimmed.description
        guard extensionType == "SessionPropertyValues"
                || extensionType == "FoundationModels.SessionPropertyValues" else {
            context.diagnose(Diagnostic(
                node: node,
                message: FoundationModelsDiagnostic("'@SessionPropertyEntry' macro can only attach to var declarations inside extensions of SessionPropertyValues"),
                highlights: [Syntax(declaration)]
            ))
            return []
        }
        guard let property = sessionProperty(
            from: declaration,
            node: node,
            requireDefaultValue: true,
            in: context
        ), let initialValue = property.initialValue else { return [] }
        let type = property.type.map { ": \($0.trimmed)" } ?? ""
        return [
            """
            private struct __Key_\(property.name): FoundationModels.SessionPropertyKey {
                @FoundationModels.__SessionPropertyEntryDefaultValue
                static var defaultValue\(raw: type) = \(initialValue.trimmed)
            }
            """
        ]
    }

    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let property = sessionProperty(
            from: declaration,
            node: node,
            requireDefaultValue: false,
            in: context
        ) else { return [] }
        return [
            """
            get {
                self[__Key_\(property.name).self]
            }
            """,
            """
            set {
                self[__Key_\(property.name).self] = newValue
            }
            """,
            """
            _modify {
                yield &self[__Key_\(property.name).self]
            }
            """,
        ]
    }
}

struct SessionPropertyEntryDefaultValueMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let variable = declaration.as(VariableDeclSyntax.self),
              let value = variable.bindings.first?.initializer?.value else {
            return []
        }
        return [
            """
            get {
                \(value.trimmed)
            }
            """
        ]
    }
}

private struct SessionProperty {
    var name: TokenSyntax
    var type: TypeSyntax?
    var initialValue: ExprSyntax?
}

private func sessionProperty(
    from declaration: some DeclSyntaxProtocol,
    node: AttributeSyntax,
    requireDefaultValue: Bool,
    in context: some MacroExpansionContext
) -> SessionProperty? {
    guard let variable = declaration.as(VariableDeclSyntax.self) else { return nil }
    if variable.bindingSpecifier.tokenKind == .keyword(.let) {
        let token = variable.bindingSpecifier
        context.diagnose(Diagnostic(
            node: node,
            message: FoundationModelsDiagnostic("'@SessionPropertyEntry' can only be applied to a 'var' declaration"),
            highlights: [Syntax(variable)],
            fixIts: [FixIt(
                message: FoundationModelsDiagnostic("Replace 'let' with 'var'"),
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
        return nil
    }
    if !requireDefaultValue, let modifier = variable.modifiers.first(where: {
        $0.name.tokenKind == .keyword(.static) || $0.name.tokenKind == .keyword(.class)
    }) {
        context.diagnose(Diagnostic(
            node: modifier.name,
            message: FoundationModelsDiagnostic("'@SessionPropertyEntry' cannot be applied to a static member"),
            fixIts: [FixIt(
                message: FoundationModelsDiagnostic("Remove 'static'"),
                changes: [.replaceText(
                    range: modifier.position..<modifier.endPosition,
                    with: "",
                    in: Syntax(variable)
                )]
            )]
        ))
        return nil
    }
    guard variable.bindings.count == 1,
          let binding = variable.bindings.first else {
        context.diagnose(Diagnostic(
            node: node,
            message: FoundationModelsDiagnostic("'@SessionPropertyEntry' can only be applied to a 'var' declaration with a simple name"),
            highlights: [Syntax(variable)]
        ))
        return nil
    }
    guard let name = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier else {
        context.diagnose(Diagnostic(
            node: binding,
            message: FoundationModelsDiagnostic("Expected an identifier for the property")
        ))
        return nil
    }
    if !requireDefaultValue, let accessorBlock = binding.accessorBlock {
        context.diagnose(Diagnostic(
            node: accessorBlock,
            message: FoundationModelsDiagnostic("'@SessionPropertyEntry' can only be applied to a stored property"),
            fixIts: [FixIt(
                message: FoundationModelsDiagnostic("Remove '@SessionPropertyEntry'"),
                changes: [.replaceText(
                    range: node.position..<node.endPosition,
                    with: "",
                    in: Syntax(variable)
                )]
            )]
        ))
        return nil
    }
    if requireDefaultValue, binding.initializer == nil {
        let position = binding.endPositionBeforeTrailingTrivia
        context.diagnose(Diagnostic(
            node: name,
            message: FoundationModelsDiagnostic("Property missing a default value"),
            highlights: [Syntax(binding)],
            fixIts: [FixIt(
                message: FoundationModelsDiagnostic("Provide default value"),
                changes: [.replaceText(
                    range: position..<position,
                    with: " = <#default value#>",
                    in: Syntax(binding)
                )]
            )]
        ))
        return nil
    }
    return SessionProperty(
        name: name.trimmed,
        type: binding.typeAnnotation?.type,
        initialValue: binding.initializer?.value
    )
}
