import FoundationMacros
import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

private struct UnsupportedParameter: Error, CustomStringConvertible {
    let description = "@Parameter requires one variable with an explicit type and default value"
}

public struct ParameterMacro: PeerMacro, AccessorMacro {
    private static func binding(_ declaration: some DeclSyntaxProtocol) throws -> PatternBindingSyntax {
        guard let variable = declaration.as(VariableDeclSyntax.self), variable.bindings.count == 1,
              let binding = variable.bindings.first, binding.pattern.is(IdentifierPatternSyntax.self),
              binding.typeAnnotation != nil, binding.initializer != nil else {
            throw UnsupportedParameter()
        }
        return binding
    }

    public static func expansion(of node: AttributeSyntax, providingPeersOf declaration: some DeclSyntaxProtocol,
                                 in context: some MacroExpansionContext) throws -> [DeclSyntax] {
        let binding = try binding(declaration)
        let name = binding.pattern.as(IdentifierPatternSyntax.self)!.identifier.text
        let variable = declaration.as(VariableDeclSyntax.self)!
        let modifiers = variable.modifiers.filter { $0.name.text != "nonisolated" }
        let options = node.arguments?.as(LabeledExprListSyntax.self)
        let suffix = options.map { $0.isEmpty ? "" : ", \($0.trimmedDescription)" } ?? ""
        return ["\(modifiers)nonisolated(unsafe) var $\(raw: name): Tips.Parameter<\(binding.typeAnnotation!.type.trimmed)> = Tips.Parameter(Self.self, \(literal: "+" + name), \(binding.initializer!.value.trimmed)\(raw: suffix))"]
    }

    public static func expansion(of node: AttributeSyntax, providingAccessorsOf declaration: some DeclSyntaxProtocol,
                                 in context: some MacroExpansionContext) throws -> [AccessorDeclSyntax] {
        let binding = try binding(declaration)
        let name = binding.pattern.as(IdentifierPatternSyntax.self)!.identifier.text
        return ["get { $\(raw: name).wrappedValue }", "set { $\(raw: name).wrappedValue = newValue }"]
    }
}

private final class UnqualifyPredicateExpressions: SyntaxRewriter {
    override func visit(_ node: MemberAccessExprSyntax) -> ExprSyntax {
        if node.base?.as(DeclReferenceExprSyntax.self)?.baseName.text == "PredicateExpressions" {
            return super.visit(node.with(\.declName.moduleSelector, nil))
        }
        if node.base?.as(DeclReferenceExprSyntax.self)?.baseName.text == "Foundation",
           node.declName.baseName.text == "PredicateExpressions" {
            return ExprSyntax(DeclReferenceExprSyntax(baseName: .identifier("PredicateExpressions")))
        }
        return super.visit(node)
    }
}

public struct RuleMacro: SwiftSyntaxMacros.ExpressionMacro {
    public static func expansion(of node: some FreestandingMacroExpansionSyntax,
                                 in context: some MacroExpansionContext) throws -> ExprSyntax {
        let predicate = try PredicateMacro.expansion(of: node, in: context)
        guard let closure = predicate.as(FunctionCallExprSyntax.self)?.arguments.first?.expression.as(ClosureExprSyntax.self) else {
            throw UnsupportedParameter()
        }
        let rewritten = UnqualifyPredicateExpressions().rewrite(closure).cast(ClosureExprSyntax.self)
        return "Tips.Rule(\(node.arguments)) \(rewritten)"
    }
}

public let tipKitMacros: [Macro.Type] = [ParameterMacro.self, RuleMacro.self]
