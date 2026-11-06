import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

/// `@ModelActor`: an actor with its own model context, which it runs on.
public struct PersistentModelActorMacro: MemberMacro, ExtensionMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingMembersOf declaration: some DeclGroupSyntax,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let access = accessPrefix(declaration.modifiers)
        return [
            "\(raw: access)nonisolated let modelExecutor: any SwiftData.ModelExecutor",
            "\(raw: access)nonisolated let modelContainer: SwiftData.ModelContainer",
            """
            \(raw: access)init(modelContainer: SwiftData.ModelContainer) {
                let modelContext = ModelContext(modelContainer)
                self.modelExecutor = DefaultSerialModelExecutor(modelContext: modelContext)
                self.modelContainer = modelContainer
            }
            """,
        ]
    }

    public static func expansion(
        of node: AttributeSyntax,
        attachedTo declaration: some DeclGroupSyntax,
        providingExtensionsOf type: some TypeSyntaxProtocol,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [ExtensionDeclSyntax] {
        guard !protocols.isEmpty else { return [] }
        let decl: DeclSyntax = "extension \(raw: type.trimmedDescription): SwiftData.ModelActor {\n}"
        return decl.as(ExtensionDeclSyntax.self).map { [$0] } ?? []
    }
}

/// `@Query`: a SwiftUI view's property whose value is the models a fetch
/// finds, kept in a `Query` made from the macro's arguments.
public struct QueryMacro: AccessorMacro, PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        let (name, _) = try queryBinding(declaration)
        return ["get {\n    _\(raw: name).wrappedValue\n}"]
    }

    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let (name, binding) = try queryBinding(declaration)
        guard let type = binding.typeAnnotation?.type.trimmedDescription else {
            throw MacroError("@Query requires a type annotation, like '[Item]'")
        }
        let arguments = node.arguments?.trimmedDescription ?? ""
        return [
            "private(set) var _\(raw: name): SwiftData.Query<\(raw: type).Element, \(raw: type)> = .init(\(raw: arguments))"
        ]
    }

    private static func queryBinding(_ declaration: some DeclSyntaxProtocol) throws -> (String, PatternBindingSyntax) {
        guard let variable = declaration.as(VariableDeclSyntax.self),
            let binding = variable.bindings.first,
            let pattern = binding.pattern.as(IdentifierPatternSyntax.self)
        else {
            throw MacroError("@Query can only be applied to a property")
        }
        return (pattern.identifier.text.trimmingCharacters(in: ["`"]), binding)
    }
}
