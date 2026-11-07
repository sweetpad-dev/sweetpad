import OpenAppleMacrosBase
import SwiftDiagnostics

struct AnimatableValuesMacro: MemberMacro, ExtensionMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingMembersOf declaration: some DeclGroupSyntax,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        if declaration.hasAnimatableData {
            context.diagnose(Diagnostic(node: node, message: AnimatableDiagnostic(
                "'@Animatable' macro has no effect on types with 'animatableData' property.",
                severity: .warning
            ), notes: [Note(node: Syntax(node), message: AnimatableNote("Remove '@Animatable'"))]))
            return []
        }

        let properties = declaration.animatableProperties(in: context)
        guard !properties.isEmpty else {
            context.diagnose(Diagnostic(node: node, message: AnimatableDiagnostic(
                "'@Animatable' macro has no effect; it can only attach to types with animatable properties."
            ), notes: [Note(node: Syntax(node), message: AnimatableNote("Remove '@Animatable'"))]))
            return []
        }

        let name = context.makeUniqueName("_animatableData").text
        let file = context.location(of: node, at: .afterLeadingTrivia, filePathMode: .filePath)?.file.description ?? "\"\""
        let payload = AnimatableContext(
            animatableDataName: name,
            fileName: file,
            varDecls: properties.map { ["name": $0.name, "line": $0.line] }
        )
        let json = String(decoding: try JSON.encode(payload), as: UTF8.self)

        return [
            """
            #_SwiftUIAnimatableDataProperty(animatableMacroContext: #"\(raw: json)"#, kind: SwiftUICore._animatableMacroKind())

            nonisolated var animatableData: some VectorArithmetic {
                get {
                    \(raw: name)
                }
                set {
                    nonisolated func inferType<T, U>(_ t: T) -> U {
                        t as! U
                    }
                    \(raw: name) = inferType(newValue)
                }
            }
            """
        ]
    }

    static func expansion(
        of node: AttributeSyntax,
        attachedTo declaration: some DeclGroupSyntax,
        providingExtensionsOf type: some TypeSyntaxProtocol,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [ExtensionDeclSyntax] {
        guard !declaration.hasAnimatableData else { return [] }
        return [try ExtensionDeclSyntax("extension \(type.trimmed): nonisolated SwiftUICore.Animatable {}")]
    }
}

struct AnimatableIgnoredMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        let enclosingType = context.lexicalContext.first { $0.asProtocol(DeclGroupSyntax.self) != nil }?
            .asProtocol(DeclGroupSyntax.self)
        let insideAnimatable = enclosingType?.attributes.contains {
            $0.as(AttributeSyntax.self)?.macroName == "Animatable"
        } ?? false
        if !insideAnimatable {
            context.diagnose(Diagnostic(node: declaration, message: AnimatableDiagnostic(
                "'@AnimatableIgnored' macro has no effect outside of an '@Animatable' type.",
                severity: .warning
            ), highlights: [Syntax(declaration)], notes: [Note(node: Syntax(declaration), message: AnimatableNote("Remove '@AnimatableIgnored'"))]))
        }
        return []
    }
}

struct AnimatableValuesDataPropertyMacro: DeclarationMacro {
    static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        try dataPropertyExpansion(of: node, pair: false)
    }
}

struct AnimatablePairDataPropertyMacro: DeclarationMacro {
    static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        try dataPropertyExpansion(of: node, pair: true)
    }
}

private func dataPropertyExpansion(
    of node: some FreestandingMacroExpansionSyntax,
    pair: Bool
) throws -> [DeclSyntax] {
        guard let string = node.arguments.first?.expression.as(StringLiteralExprSyntax.self),
              case .stringSegment(let segment) = string.segments.first else {
            return []
        }
        let payload = try Array(segment.content.text.utf8).withUnsafeBufferPointer {
            try JSON.decode(AnimatableContext.self, from: $0)
        }
        let name = payload.animatableDataName
        let file = payload.fileName
        let properties = payload.varDecls
        let lines = properties.compactMap { property -> String? in
            guard let name = property["name"], let line = property["line"] else { return nil }
            return """
                #sourceLocation(file: \(file), line: \(line))
                let \(name) = #_SwiftUIAnimatableProperty(Self[_animatableType: \\.\(name)])
                #sourceLocation()
                """
        }.joined(separator: "\n")
        let values = properties.compactMap { $0["name"] }
        guard !values.isEmpty else { return [] }
        let result: String
        if pair {
            result = pairExpression(values, indent: 4, zeroLeaves: true)
        } else {
            result = "SwiftUICore.AnimatableValues(\(values.joined(separator: ", ")))"
        }
        let attribute = pair ? "_AnimatablePairData" : "_AnimatableData"
        return [
            """
            @SwiftUICore.\(raw: attribute)
            private nonisolated var \(raw: name) = {
                \(raw: lines)
                return \(raw: result)
            }()
            """
        ]
}

struct AnimatableValuesDataMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        dataAccessorExpansion(of: declaration, pair: false)
    }
}

struct AnimatablePairDataMacro: AccessorMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingAccessorsOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [AccessorDeclSyntax] {
        dataAccessorExpansion(of: declaration, pair: true)
    }
}

private func dataAccessorExpansion(
    of declaration: some DeclSyntaxProtocol,
    pair: Bool
) -> [AccessorDeclSyntax] {
        guard let variable = declaration.as(VariableDeclSyntax.self),
              let call = variable.bindings.first?.initializer?.value.as(FunctionCallExprSyntax.self),
              let closure = call.calledExpression.as(ClosureExprSyntax.self) else { return [] }
        let names = closure.statements.compactMap {
            $0.item.as(VariableDeclSyntax.self)?.bindings.first?.pattern.as(IdentifierPatternSyntax.self)?.identifier.text
        }
        guard !names.isEmpty else { return [] }
        let getters = names.map { "let \($0) = self[_animatableValue: \\.\($0)]" }.joined(separator: "\n")
        let setters = names.enumerated().map { index, name in
            let value = pair ? pairPath(index, count: names.count) :
                "newValue.value\(names.count == 1 ? "" : ".\(index)")"
            return "self[_animatableValue: \\.\(name)] = \(value)"
        }.joined(separator: "\n")
        let result = pair ? pairExpression(names, indent: 4, zeroLeaves: false) :
            "SwiftUICore.AnimatableValues(\(names.joined(separator: ", ")))"
        return [
            """
            get {
                \(raw: getters)
                return \(raw: result)
            }
            """,
            """
            set {
                \(raw: setters)
            }
            """,
        ]
}

private func pairExpression(
    _ values: [String],
    indent: Int,
    zeroLeaves: Bool,
    nested: Bool = false
) -> String {
    if values.count == 1 {
        return values[0] + (zeroLeaves ? ".zero" : "")
    }
    let middle = values.count / 2
    let zeroChild = zeroLeaves && values.count > 2
    let nestedIndent = indent + (nested ? 0 : 4)
    let left = pairExpression(Array(values[..<middle]), indent: nestedIndent, zeroLeaves: zeroChild, nested: true)
    let right = pairExpression(Array(values[middle...]), indent: nestedIndent, zeroLeaves: zeroChild, nested: true)
    let padding = String(repeating: " ", count: indent)
    let childPadding = nested ? padding : padding + "    "
    return "SwiftUICore.AnimatablePair(\n\(childPadding)\(left),\n\(childPadding)\(right)\n\(padding))"
}

private func pairPath(_ index: Int, count: Int) -> String {
    guard count > 1 else { return "newValue" }
    let middle = count / 2
    if index < middle {
        return "newValue.first" + String(pairPath(index, count: middle).dropFirst("newValue".count))
    } else {
        return "newValue.second" + String(pairPath(index - middle, count: count - middle).dropFirst("newValue".count))
    }
}

struct AnimatablePropertyMacro: ExpressionMacro {
    static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> ExprSyntax {
        node.arguments.first?.expression ?? "()"
    }
}

struct InvalidAnimatablePropertyMacro: ExpressionMacro {
    static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> ExprSyntax {
        context.diagnose(Diagnostic(node: node, message: AnimatableDiagnostic(
            "Cannot automatically synthesize 'animatableData'."
        ), notes: [
            Note(node: Syntax(node), message: AnimatableNote("Mark this property with '@AnimatableIgnored'.")),
            Note(node: Syntax(node), message: AnimatableNote("Conform the type of this property to 'Animatable' or 'VectorArithmetic'.")),
        ]))
        return "EmptyAnimatableData.self"
    }
}

private struct AnimatableContext: Codable {
    let animatableDataName: String
    let fileName: String
    let varDecls: [[String: String]]
}

private struct AnimatableProperty {
    let name: String
    let line: String
}

private extension DeclGroupSyntax {
    var hasAnimatableData: Bool {
        memberBlock.members.contains { member in
            guard let variable = member.decl.as(VariableDeclSyntax.self) else { return false }
            return variable.bindings.contains {
                $0.pattern.as(IdentifierPatternSyntax.self)?.identifier.text == "animatableData"
            }
        }
    }

    func animatableProperties(in context: some MacroExpansionContext) -> [AnimatableProperty] {
        memberBlock.members.flatMap { member -> [AnimatableProperty] in
            guard let variable = member.decl.as(VariableDeclSyntax.self),
                  variable.bindingSpecifier.tokenKind == .keyword(.var),
                  !variable.attributes.contains(where: {
                      $0.as(AttributeSyntax.self)?.macroName == "AnimatableIgnored"
                  }) else { return [] }
            return variable.bindings.compactMap { binding in
                guard binding.isStored,
                      let identifier = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier,
                      let location = context.location(of: binding, at: .afterLeadingTrivia, filePathMode: .filePath) else {
                    return nil
                }
                return AnimatableProperty(name: identifier.text, line: location.line.description)
            }
        }
    }
}

private extension PatternBindingSyntax {
    var isStored: Bool {
        guard let accessorBlock else { return true }
        guard case .accessors(let accessors) = accessorBlock.accessors else { return false }
        return accessors.allSatisfy {
            $0.accessorSpecifier.tokenKind == .keyword(.willSet) ||
            $0.accessorSpecifier.tokenKind == .keyword(.didSet)
        }
    }
}

private struct AnimatableDiagnostic: DiagnosticMessage {
    let message: String
    let severity: DiagnosticSeverity

    init(_ message: String, severity: DiagnosticSeverity = .error) {
        self.message = message
        self.severity = severity
    }

    var diagnosticID: MessageID { MessageID(domain: "SwiftUIMacros", id: message) }
}

private struct AnimatableNote: NoteMessage {
    let message: String

    init(_ message: String) { self.message = message }
    var noteID: MessageID { MessageID(domain: "SwiftUIMacros", id: message) }
}

private extension AttributeSyntax {
    var macroName: String? {
        attributeName.trimmed.description.split(separator: ".").last.map(String.init)
    }
}
