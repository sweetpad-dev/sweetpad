import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

/// `@Model`: makes a class a SwiftData model. It adds the backing data and
/// the schema's metadata, marks each stored property persisted, and adds
/// the `PersistentModel` and `Observable` conformances. A subclass of
/// another model, which the compiler doesn't ask to conform again, gets
/// its own backing data and passes its parent the rest.
public struct PersistentModelMacro: MemberMacro, MemberAttributeMacro, ExtensionMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingMembersOf declaration: some DeclGroupSyntax,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        guard let model = declaration.as(ClassDeclSyntax.self) else {
            throw MacroError("@Model can only be applied to a class")
        }
        let name = model.name.text
        let access = accessPrefix(model.modifiers)
        let subclass = !protocols.contains { $0.trimmedDescription.hasSuffix("PersistentModel") }
        let properties = model.persistedProperties

        var members: [DeclSyntax] = [
            """
            @Transient
            private var _$backingData: any SwiftData.BackingData<\(raw: name)> = \(raw: name).createBackingData()
            """
        ]
        if !subclass {
            members.append(
                """
                public var persistentBackingData: any SwiftData.BackingData<\(raw: name)> {
                    get {
                        return _$backingData
                    }
                    set {
                        _$backingData = newValue
                    }
                }
                """
            )
        }
        members.append(DeclSyntax(stringLiteral: schemaMetadata(model, properties, access: access, subclass: subclass)))
        let stores = properties.map { "  _\($0.name) = _SwiftDataNoType()" }.joined(separator: "\n")
        if subclass {
            members.append(
                """
                required init(backingData: any SwiftData.BackingData<Root>) {
                \(raw: stores)
                  _$backingData = backingData._generateCurrentClassBackingData()
                  super.init(backingData: backingData._superClassBackingData(of: \(raw: name).self))
                }
                """
            )
        } else {
            let final = model.modifiers.contains { $0.name.text == "final" }
            let required = final ? "" : "required "
            members.append(
                """
                \(raw: access)\(raw: required)init(backingData: any SwiftData.BackingData<\(raw: name)>) {
                \(raw: stores)
                  self.persistentBackingData = backingData
                }
                """
            )
        }
        members.append(contentsOf: [
            "@Transient private let _$observationRegistrar = Observation.ObservationRegistrar()",
            """
            internal nonisolated func access<_M>(
                keyPath: KeyPath<\(raw: name), _M>
            ) {
              _$observationRegistrar.access(self, keyPath: keyPath)
            }
            """,
            """
            internal nonisolated func withMutation<_M, _MR>(
              keyPath: KeyPath<\(raw: name), _M>,
              _ mutation: () throws -> _MR
            ) rethrows -> _MR {
              try _$observationRegistrar.withMutation(of: self, keyPath: keyPath, mutation)
            }
            """,
            """
            struct _SwiftDataNoType {
            }
            """,
        ])
        return members
    }

    /// `schemaMetadata`: each persisted property's name, key path, default
    /// and options, and the model's indexes and uniqueness constraints,
    /// which need iOS 18.
    static func schemaMetadata(
        _ model: ClassDeclSyntax,
        _ properties: [StoredProperty],
        access: String,
        subclass: Bool
    ) -> String {
        let name = model.name.text
        let entries = properties.map { property in
            let metadata: String
            if let attribute = property.attributes.attribute(named: "Attribute") {
                metadata = "SwiftData.Schema.Attribute\(arguments(attribute))"
            } else if let relationship = property.attributes.attribute(named: "Relationship") {
                metadata = "SwiftData.Schema.Relationship\(arguments(relationship))"
            } else {
                metadata = "nil"
            }
            return "    SwiftData.Schema.PropertyMetadata(name: \"\(property.name)\", keypath: \\\(name).\(property.spelling), "
                + "defaultValue: \(property.initializer ?? "nil"), metadata: \(metadata))"
        }.joined(separator: ",\n")

        // The indexes, then the uniqueness constraints, each in a block of
        // its own, empty when the model has none.
        var others: [String] = []
        for (macro, type) in [("Index", "SwiftData.Schema.Index"), ("Unique", "SwiftData.Schema.Unique")] {
            var blocks: [String] = []
            for member in model.memberBlock.members {
                guard let expansion = member.decl.as(MacroExpansionDeclSyntax.self),
                    expansion.macroName.text == macro
                else { continue }
                let generics = expansion.genericArgumentClause?.trimmedDescription ?? ""
                blocks.append(
                    """
                      if #available(macOS 15, iOS 18, tvOS 18, watchOS 11, visionOS 2, *) {
                        otherProperties.append(
                          SwiftData.Schema.PropertyMetadata(name: "\(type)", keypath: \\SwiftData.Schema.encodingVersion, defaultValue: nil, metadata: \(type)\(generics)(\(expansion.arguments.trimmedDescription))))
                      }
                    """
                )
            }
            others.append(blocks.joined(separator: "\n"))
        }
        let override = subclass ? "override " : ""
        let header = "\(access)\(override)class var schemaMetadata: [SwiftData.Schema.PropertyMetadata] {"
        if others.allSatisfy(\.isEmpty) {
            return "\(header)\n  return [\n\(entries)\n  ]\n}"
        }
        return """
            \(header)
              let storedProperties = [
            \(entries)
              ]
              var otherProperties = [SwiftData.Schema.PropertyMetadata]()
            \(others.joined(separator: "\n"))
              return storedProperties + otherProperties
            }
            """
    }

    /// An attribute's arguments in parentheses, as `@Model` passes them on:
    /// ` ()` when it has none.
    static func arguments(_ attribute: AttributeSyntax) -> String {
        guard let arguments = attribute.arguments else { return " ()" }
        return "(\(arguments.trimmedDescription))"
    }

    public static func expansion(
        of node: AttributeSyntax,
        attachedTo declaration: some DeclGroupSyntax,
        providingAttributesFor member: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AttributeSyntax] {
        guard let variable = member.as(VariableDeclSyntax.self), !variable.isStatic,
            variable.attributes.attribute(named: "Transient") == nil,
            !variable.storedBindings.isEmpty
        else { return [] }
        let transformable = variable.attributes.attribute(named: "Attribute")?.arguments?
            .trimmedDescription.contains(".transformable(") ?? false
        return [transformable ? "@_TransformablePersistedProperty" : "@_PersistedProperty"]
    }

    public static func expansion(
        of node: AttributeSyntax,
        attachedTo declaration: some DeclGroupSyntax,
        providingExtensionsOf type: some TypeSyntaxProtocol,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [ExtensionDeclSyntax] {
        let names = protocols.map(\.trimmedDescription)
        let type = type.trimmedDescription
        // The conformances are only as available as the model.
        let availability = declaration.attributes.compactMap { element -> String? in
            guard case .attribute(let attribute) = element,
                attribute.attributeName.trimmedDescription == "available"
            else { return nil }
            return attribute.trimmedDescription + "\n"
        }.joined()
        var extensions: [DeclSyntax] = []
        if names.contains(where: { $0.hasSuffix("PersistentModel") }) {
            extensions.append("\(raw: availability)extension \(raw: type): nonisolated SwiftData.PersistentModel {\n}")
        }
        if names.contains(where: { $0.hasSuffix("Observable") }) {
            extensions.append("\(raw: availability)extension \(raw: type): nonisolated Observation.Observable {\n}")
        }
        if names.contains(where: { $0.hasSuffix("Sendable") }) {
            let message = "PersistentModels are not Sendable, consider utilizing a ModelActor or use \(type)'s persistentModelID instead"
            extensions.append(
                """
                @available(swift, deprecated: 5.9, message: "\(raw: message)")
                @available(*, unavailable, message: "\(raw: message)")
                extension \(raw: type): Sendable {
                }
                """
            )
        }
        return extensions.compactMap { $0.as(ExtensionDeclSyntax.self) }
    }
}

/// `@_PersistedProperty`, which `@Model` puts on each stored property: the
/// property reads and writes the model's backing data, telling the
/// observation registrar, and its storage becomes a placeholder.
public struct PersistedPropertyMacro: AccessorMacro, PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        try persistedAccessors(declaration, getter: "getValue", setter: "setValue")
    }

    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        try placeholder(declaration)
    }
}

/// `@_TransformablePersistedProperty`, for a property whose `@Attribute`
/// names a value transformer.
public struct TransformablePersistedPropertyMacro: AccessorMacro, PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        try persistedAccessors(declaration, getter: "getTransformableValue", setter: "setTransformableValue")
    }

    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        try placeholder(declaration)
    }
}

private func persistedBinding(_ declaration: some DeclSyntaxProtocol) throws -> (String, PatternBindingSyntax) {
    guard let variable = declaration.as(VariableDeclSyntax.self),
        let first = variable.storedBindings.first
    else {
        throw MacroError("a persisted property must be a stored property")
    }
    return first
}

/// Whether the property is a `let`, which gets no setter.
private func isConstant(_ declaration: some DeclSyntaxProtocol) -> Bool {
    declaration.as(VariableDeclSyntax.self)?.bindingSpecifier.text == "let"
}

private func persistedAccessors(
    _ declaration: some DeclSyntaxProtocol,
    getter: String,
    setter: String
) throws -> [AccessorDeclSyntax] {
    let (name, binding) = try persistedBinding(declaration)
    let spelling = binding.pattern.trimmedDescription
    var accessors: [AccessorDeclSyntax] = [
        """
        @storageRestrictions(accesses: _$backingData, initializes: _\(raw: name))
        init(initialValue) {
            _$backingData.\(raw: setter)(forKey: \\.\(raw: spelling), to: initialValue)
            _\(raw: name) = _SwiftDataNoType()
        }
        """,
        """
        get {
            _$observationRegistrar.access(self, keyPath: \\.\(raw: spelling))
            return self.\(raw: getter)(forKey: \\.\(raw: spelling))
        }
        """
    ]
    if isConstant(declaration) {
        return accessors
    }
    accessors.append(
        """
        set {
            _$observationRegistrar.withMutation(of: self, keyPath: \\.\(raw: spelling)) {
                self.\(raw: setter)(forKey: \\.\(raw: spelling), to: newValue)
            }
        }
        """
    )
    return accessors
}

/// The storage a persisted property keeps in place of its value: optional
/// when the property's type is spelled optional, and initialized when the
/// property has a default.
private func placeholder(_ declaration: some DeclSyntaxProtocol) throws -> [DeclSyntax] {
    let (name, binding) = try persistedBinding(declaration)
    var type = "_SwiftDataNoType"
    if let annotation = binding.typeAnnotation?.type {
        if annotation.is(OptionalTypeSyntax.self) {
            type += "?"
        } else if annotation.is(ImplicitlyUnwrappedOptionalTypeSyntax.self) {
            type += "!"
        }
    }
    let initializer = binding.initializer == nil ? "" : " = _SwiftDataNoType()"
    return ["@Transient\nprivate var _\(raw: name): \(raw: type)\(raw: initializer)"]
}

/// `@Attribute`, `@Relationship` and `@Transient` mark a property for
/// `@Model`, which reads them; on their own they add nothing.
public struct AttributePropertyMacro: PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        []
    }
}

public struct RelationshipPropertyMacro: PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        []
    }
}

public struct TransientPropertyMacro: PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        []
    }
}

/// `#Unique` and `#Index` in a model's body, which `@Model` reads for the
/// schema; on their own they add nothing.
public struct UniqueConstraintsMacro: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        []
    }
}

public struct IndexMacro: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        []
    }
}
