import OpenAppleMacrosBase

struct StateMacro: PeerMacro, AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        guard let variable = declaration.as(VariableDeclSyntax.self),
              variable.bindings.count == 1,
              let binding = variable.bindings.first,
              let identifier = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier else {
            return []
        }

        let name = identifier.trimmed.text
        let type = binding.typeAnnotation?.type.trimmed.description
        let isPrivate = variable.modifiers.contains { $0.name.tokenKind == .keyword(.private) }
        let projectedModifier = variable.modifiers.first.map { "\($0.name.trimmed) " } ?? ""
        let explicitValue = binding.initializer?.value.trimmed.description ?? node.stateValueArgument
        let value = explicitValue
            ?? (binding.typeAnnotation?.type.as(OptionalTypeSyntax.self) != nil ? "nil" : nil)

        guard let value else {
            guard let type else { return [] }
            return [
                "private var _\(raw: name): SwiftUICore.State<\(raw: type)>",
                """
                \(raw: projectedModifier)var $\(raw: name): SwiftUICore.Binding<\(raw: type)> {
                    _\(raw: name).projectedValue
                }
                """,
            ]
        }

        let stateType = type.map { "<\($0)>" } ?? ""
        let state = "SwiftUICore.State\(stateType)(initialValue: \(value))"

        if !isPrivate {
            return [
                "private var _\(raw: name) = \(raw: state)",
                """
                @SwiftUICore._PropertyWrapperProjectedValue
                \(raw: projectedModifier)var $\(raw: name) = \(raw: state).projectedValue
                """,
            ]
        }

        // Private state uses lazy backing storage and forwards initialization through helper macros.
        let initialStoredName = context.makeUniqueName("_initialStoredValue_").text
        let initialStoredNameEncoded = initialStoredName.utf8.base64EncodedString()
        let storage: String
        let initialStoredValue: String
        let initialStoredGetter: String
        if let type {
            storage = "SwiftUICore.State._makeStorage(({ let value: \(type) = \(value)\nreturn value }))"
            initialStoredValue = "SwiftUICore.State._makeStorage(initialValue: { let x: \(type) = \(value)\nreturn x }())"
            initialStoredGetter = explicitValue == nil ? initialStoredValue : storage
        } else {
            storage = "SwiftUICore.State._makeStorage({ \(value) })"
            initialStoredValue = "SwiftUICore.State._makeStorage(initialValue: \(value))"
            initialStoredGetter = storage
        }
        // The helper macro receives the getter expression as base64 to preserve its source text.
        let initialStoredValueEncoded = initialStoredGetter.utf8.base64EncodedString()
        let stateExpression = ExprSyntax("\(raw: state)")

        return [
            "private var __\(raw: name) = \(raw: storage)",
            "@SwiftUICore._StatePropertyWrapperStorage(initialValue: \"\(raw: initialStoredNameEncoded)\")\nprivate var _\(raw: name): SwiftUICore.State<_>! = SwiftUICore._stateNil(of: {\n        \(stateExpression)\n    })",
            """
            @SwiftUICore._StateProjectedValue
            private var $\(raw: name) = \(raw: state).projectedValue
            """,
            """
            @SwiftUICore._StateInitialStoredValue("\(raw: initialStoredValueEncoded)")
            private static var \(raw: initialStoredName) = (\(raw: initialStoredValue))
            """,
        ]
    }

    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let variable = declaration.as(VariableDeclSyntax.self),
              variable.bindings.count == 1,
              let binding = variable.bindings.first,
              let identifier = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier else {
            return []
        }

        let name = identifier.trimmed.text
        let isPrivate = variable.modifiers.contains { $0.name.tokenKind == .keyword(.private) }
        let hasExplicitValue = binding.initializer != nil || node.stateValueArgument != nil
        let hasValue = hasExplicitValue || binding.typeAnnotation?.type.as(OptionalTypeSyntax.self) != nil
        let storage = isPrivate && hasValue ? "__\(name)" : "_\(name)"
        var accessors: [AccessorDeclSyntax] = []
        if !isPrivate || !hasExplicitValue {
            accessors.append(
                """
                @storageRestrictions(initializes: \(raw: storage))
                init(initialValue) {
                    \(raw: storage) = SwiftUICore.State\(raw: isPrivate && hasValue ? "._makeStorage" : "")(initialValue: initialValue)
                }
                """
            )
        }
        accessors.append("get { \(raw: storage).wrappedValue }")
        accessors.append("nonmutating set { \(raw: storage).wrappedValue = newValue }")
        return accessors
    }
}

struct ProjectedValueMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let name = statePropertyName(declaration), name.hasPrefix("$") else { return [] }
        let wrappedName = String(name.dropFirst())
        return ["get { _\(raw: wrappedName).projectedValue }"]
    }
}

struct StateProjectedValueMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let name = statePropertyName(declaration), name.hasPrefix("$") else { return [] }
        let wrappedName = String(name.dropFirst())
        return ["get { __\(raw: wrappedName).projectedValue }"]
    }
}

struct StatePropertyWrapperStorageMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let name = statePropertyName(declaration),
              let encodedName = node.stateStringArgument,
              let nameBytes = [UInt8](base64EncodedString: encodedName) else {
            return []
        }
        let initialStoredName = String(decoding: nameBytes, as: UTF8.self)
        guard initialStoredName.utf8.elementsEqual(nameBytes) else { return [] }
        let storage = "__\(name.dropFirst())"
        return [
            """
            @storageRestrictions(initializes: \(raw: storage))
            init(initialValue) {
                if initialValue == nil {
                    \(raw: storage) = Self.\(raw: initialStoredName)
                } else {
                    \(raw: storage) = SwiftUICore.State._makeStorage(initialValue: initialValue.wrappedValue)
                }
            }
            """,
            "get { SwiftUICore.State(initialValue: \(raw: storage).wrappedValue) }",
            """
            set {
                if newValue != nil {
                    \(raw: storage) = SwiftUICore.State._makeStorage(initialValue: newValue.wrappedValue)
                }
            }
            """,
        ]
    }
}

struct StateInitialStoredValueMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        guard let encoded = node.stateStringArgument,
              let bytes = [UInt8](base64EncodedString: encoded) else {
            return []
        }
        let expression = String(decoding: bytes, as: UTF8.self)
        guard expression.utf8.elementsEqual(bytes) else { return [] }
        return ["get { \(raw: expression) }"]
    }
}

private func statePropertyName(_ declaration: some DeclSyntaxProtocol) -> String? {
    guard let variable = declaration.as(VariableDeclSyntax.self),
          let identifier = variable.bindings.first?.pattern.as(IdentifierPatternSyntax.self)?.identifier else {
        return nil
    }
    return identifier.trimmed.text
}

private extension AttributeSyntax {
    var stateValueArgument: String? {
        guard case .argumentList(let arguments) = arguments,
              let argument = arguments.first,
              let label = argument.label?.text,
              label == "initialValue" || label == "wrappedValue" else {
            return nil
        }
        return argument.expression.trimmed.description
    }

    var stateStringArgument: String? {
        guard case .argumentList(let arguments) = arguments,
              let string = arguments.first?.expression.as(StringLiteralExprSyntax.self),
              string.segments.count == 1,
              case .stringSegment(let segment) = string.segments.first else {
            return nil
        }
        return segment.content.text
    }
}
