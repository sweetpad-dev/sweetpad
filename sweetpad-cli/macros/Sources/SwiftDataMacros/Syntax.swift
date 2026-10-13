import SwiftSyntax
import SwiftSyntaxMacros

/// The macros of Apple's `SwiftDataMacros` plugin, which SwiftData and its
/// SwiftUI overlay declare.
public let swiftDataMacros: [Macro.Type] = [
    PersistentModelMacro.self,
    PersistentModelActorMacro.self,
    PersistedPropertyMacro.self,
    TransformablePersistedPropertyMacro.self,
    AttributePropertyMacro.self,
    RelationshipPropertyMacro.self,
    TransientPropertyMacro.self,
    UniqueConstraintsMacro.self,
    IndexMacro.self,
    QueryMacro.self,
]

/// A message the compiler shows where a macro is used wrong.
struct MacroError: Error, CustomStringConvertible {
    let description: String

    init(_ description: String) {
        self.description = description
    }
}

extension AttributeListSyntax {
    /// The attribute named `name`, as `@name` or `@SwiftData.name`.
    func attribute(named name: String) -> AttributeSyntax? {
        for element in self {
            guard case .attribute(let attribute) = element else { continue }
            let type = attribute.attributeName.trimmedDescription
            if type == name || type == "SwiftData.\(name)" {
                return attribute
            }
        }
        return nil
    }
}

/// The access level the members a macro adds get: `public` for a public
/// or open declaration, else the default.
func accessPrefix(_ modifiers: DeclModifierListSyntax) -> String {
    let public_ = modifiers.contains { ["public", "open"].contains($0.name.text) }
    return public_ ? "public " : ""
}

/// A stored property of a model, as `@Model` sees it.
struct StoredProperty {
    let name: String
    let binding: PatternBindingSyntax
    let attributes: AttributeListSyntax

    /// The property's name as it's written, backticks and all.
    var spelling: String {
        binding.pattern.trimmedDescription
    }

    var initializer: String? {
        binding.initializer?.value.trimmedDescription
    }
}

extension VariableDeclSyntax {
    var isStatic: Bool {
        modifiers.contains { ["static", "class"].contains($0.name.text) }
    }

    /// The names this declares that hold a value of their own: not those
    /// with a getter, which compute theirs.
    var storedBindings: [(String, PatternBindingSyntax)] {
        bindings.compactMap { binding in
            guard let pattern = binding.pattern.as(IdentifierPatternSyntax.self) else { return nil }
            if let accessors = binding.accessorBlock {
                switch accessors.accessors {
                case .getter:
                    return nil
                case .accessors(let list):
                    let observers = list.allSatisfy { ["willSet", "didSet"].contains($0.accessorSpecifier.text) }
                    if !observers { return nil }
                }
            }
            let name = pattern.identifier.text.trimmingCharacters(in: ["`"])
            return (name, binding)
        }
    }
}

extension DeclGroupSyntax {
    /// The stored properties `@Model` persists: every instance `var` or
    /// `let` with storage of its own, except those marked `@Transient`.
    var persistedProperties: [StoredProperty] {
        memberBlock.members.flatMap { member -> [StoredProperty] in
            guard let variable = member.decl.as(VariableDeclSyntax.self), !variable.isStatic,
                variable.attributes.attribute(named: "Transient") == nil
            else { return [] }
            return variable.storedBindings.map {
                StoredProperty(name: $0.0, binding: $0.1, attributes: variable.attributes)
            }
        }
    }
}

extension String {
    func trimmingCharacters(in set: Set<Character>) -> String {
        var text = Substring(self)
        while let first = text.first, set.contains(first) { text = text.dropFirst() }
        while let last = text.last, set.contains(last) { text = text.dropLast() }
        return String(text)
    }
}
