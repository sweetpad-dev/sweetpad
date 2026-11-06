import SwiftDiagnostics
import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

/// The macro types of Apple's `PreviewsMacros`, which the SDK's `#Preview`
/// declarations name.
public let previewsMacros: [Macro.Type] = [
    SwiftUIView.self,
    SwiftUIViewGroup_1.self,
    KitViewMacro.self,
    PreviewCommonGroup.self,
    Common.self,
    Previewable.self,
]

/// `#Preview` of a SwiftUI view: its body built as a `ViewBuilder`, or,
/// when it opens with `@Previewable` variables, as the body of a view that
/// holds them.
public struct SwiftUIView: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let preview = try previewCall(node) { closure in
            for warning in PreviewWarnings.find(in: closure) {
                context.diagnose(warning)
            }
            let (previewables, statements) = try previewableSplit(closure.statements)
            if previewables.isEmpty {
                let build: StmtSyntax = "return __b_buildView \(closure.trimmed)"
                return """
                    {
                        func __b_buildView(@SwiftUI.ViewBuilder body: () -> any SwiftUI.View) -> any SwiftUI.View {
                            body()
                        }
                        \(build)
                    }
                    """
            }
            return previewableWrapper(previewables, statements)
        }
        return [try registry(of: node, in: context, preview: preview)]
    }
}

/// `#Preview` of a SwiftUI view for each of its arguments.
public struct SwiftUIViewGroup_1: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        [try registry(of: node, in: context, preview: previewCall(node) { ExprSyntax($0) })]
    }
}

/// `#Preview` of a UIKit or AppKit view or view controller.
public struct KitViewMacro: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let preview = try previewCall(node) { closure in
            let build: StmtSyntax = "return __b_buildContent \(closure.trimmed)"
            return """
            {
                @resultBuilder struct __B_Builder<Content> {
                    static func buildBlock(_ content: Content) -> Content {
                        content
                    }
                }
                func __b_buildContent<Content>(@__B_Builder<Content> build: () -> Content) -> Content {
                    build()
                }
                \(build)
            }
            """
        }
        return [try registry(of: node, in: context, preview: preview)]
    }
}

/// `#Preview` of a UIKit or AppKit view or view controller for each of its
/// arguments.
public struct PreviewCommonGroup: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        [try registry(of: node, in: context, preview: previewCall(node) { ExprSyntax($0) })]
    }
}

/// `#Preview` of a widget, with its timeline, provider, relevance or
/// activity states.
public struct Common: DeclarationMacro {
    public static func expansion(
        of node: some FreestandingMacroExpansionSyntax,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        [try registry(of: node, in: context, preview: previewCall(node) { ExprSyntax($0) })]
    }
}

/// `@Previewable`, which `#Preview` takes off the variables it moves into
/// the view it makes; anywhere else it's an error.
public struct Previewable: PeerMacro {
    public static func expansion(
        of node: AttributeSyntax,
        providingPeersOf declaration: some DeclSyntaxProtocol,
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        let inPreview = context.lexicalContext.contains {
            $0.as(MacroExpansionDeclSyntax.self)?.macroName.text == "Preview"
                || $0.as(MacroExpansionExprSyntax.self)?.macroName.text == "Preview"
        }
        if !inPreview {
            throw error(at: node, "'@Previewable' can only be used in a SwiftUI #Preview body closure")
        }
        return []
    }
}

struct PreviewDiagnostic: DiagnosticMessage {
    let message: String
    var severity = DiagnosticSeverity.error
    var diagnosticID: MessageID { MessageID(domain: "PreviewsMacros", id: message) }
}

func error(at node: some SyntaxProtocol, _ message: String) -> DiagnosticsError {
    DiagnosticsError(diagnostics: [Diagnostic(node: node, message: PreviewDiagnostic(message: message))])
}

/// The type the preview canvas finds the preview through, named after its
/// place in the source.
func registry(
    of node: some FreestandingMacroExpansionSyntax,
    in context: some MacroExpansionContext,
    preview: ExprSyntax
) throws -> DeclSyntax {
    let name = context.makeUniqueName("PreviewRegistry")
    guard let location = context.location(of: node, at: .afterLeadingTrivia, filePathMode: .fileID) else {
        throw error(at: node, "#Preview has no source location")
    }
    return """
        @available(iOS 17.0, macOS 14.0, tvOS 17.0, visionOS 1.0, watchOS 10.0, *)
        nonisolated struct \(name): DeveloperToolsSupport.PreviewRegistry {
            static var fileID: String {
                \(location.file)
            }
            static var line: Int {
                \(location.line)
            }
            static var column: Int {
                \(location.column)
            }

            @MainActor static func makePreview() throws -> DeveloperToolsSupport.Preview {
                \(preview)
            }
        }
        """
}

/// `DeveloperToolsSupport.Preview` called with the macro's arguments and
/// closures, its body closure made into what `body` returns.
func previewCall(
    _ node: some FreestandingMacroExpansionSyntax,
    body: (ClosureExprSyntax) throws -> ExprSyntax
) throws -> ExprSyntax {
    var arguments = node.arguments
    var trailing = node.trailingClosure.map(ExprSyntax.init)
    if let closure = node.trailingClosure {
        trailing = try body(closure)
    } else if let last = arguments.last, let closure = last.expression.as(ClosureExprSyntax.self) {
        arguments[arguments.index(before: arguments.endIndex)] = last.with(\.expression, try body(closure))
    }
    let call = FunctionCallExprSyntax(
        calledExpression: ExprSyntax("DeveloperToolsSupport.Preview")
            .with(\.trailingTrivia, node.macroName.trailingTrivia),
        leftParen: node.leftParen,
        arguments: arguments,
        rightParen: node.rightParen,
        trailingClosure: trailing?.as(ClosureExprSyntax.self),
        additionalTrailingClosures: node.additionalTrailingClosures
    )
    return ExprSyntax(call)
}

/// A preview body's leading `@Previewable` variables, without the
/// attribute, and the statements after them.
func previewableSplit(
    _ statements: CodeBlockItemListSyntax
) throws -> ([VariableDeclSyntax], [CodeBlockItemSyntax]) {
    var previewables: [VariableDeclSyntax] = []
    var rest: [CodeBlockItemSyntax] = []
    for item in statements {
        if let variable = item.item.as(VariableDeclSyntax.self),
            let attribute = previewableAttribute(variable.attributes)
        {
            if !rest.isEmpty {
                throw error(at: attribute, "'@Previewable' items must be at the beginning of the preview block")
            }
            let attributes = variable.attributes.filter { $0.as(AttributeSyntax.self) != attribute }
            previewables.append(variable.with(\.attributes, attributes))
        } else {
            rest.append(item)
        }
    }
    for item in rest {
        if let nested = NestedPreviewable.first(in: item) {
            throw error(at: nested, "'@Previewable' items must be at root scope in the preview block")
        }
    }
    return (previewables, rest)
}

/// The closure a preview body that opens with `@Previewable` variables
/// becomes: a view holding them, with the rest of the body as its body.
/// Each line keeps the indentation it has in the source, moved over as
/// Xcode's plugin moves it.
func previewableWrapper(
    _ previewables: [VariableDeclSyntax],
    _ statements: [CodeBlockItemSyntax]
) -> ExprSyntax {
    let members = previewables.map { "        " + shifted($0.trimmedDescription, by: 4) }
    var body = ""
    if var first = statements.first {
        first.leadingTrivia = Trivia(pieces: first.leadingTrivia.drop(while: \.isNewline))
        body = ([first] + statements.dropFirst()).map(\.description).joined()
    }
    let text = """
        {
            struct __P_Previewable_Transform_Wrapper: SwiftUI.View {
        \(members.joined(separator: "\n\n"))

                var body: some SwiftUI.View {
                \(shifted(body, by: 8))
                }
            }
            return __P_Previewable_Transform_Wrapper()
        }
        """
    return ExprSyntax("\(raw: text)")
}

/// `text` with `width` more columns of indentation on each line after the
/// first that isn't blank.
func shifted(_ text: String, by width: Int) -> String {
    let indentation = String(repeating: " ", count: width)
    return text.split(separator: "\n", omittingEmptySubsequences: false)
        .enumerated()
        .map { index, line in index == 0 || line.isEmpty ? String(line) : indentation + line }
        .joined(separator: "\n")
}

func previewableAttribute(_ attributes: AttributeListSyntax) -> AttributeSyntax? {
    for element in attributes {
        if let attribute = element.as(AttributeSyntax.self),
            attribute.attributeName.trimmedDescription == "Previewable"
        {
            return attribute
        }
    }
    return nil
}

/// What Xcode's plugin warns about in a SwiftUI preview body: a property
/// wrapper on a local variable, which only works on a `@Previewable` one,
/// and the modifiers `PreviewProvider` used that `#Preview` ignores.
final class PreviewWarnings: SyntaxVisitor {
    /// The wrappers Xcode's plugin names; others it lets pass.
    static let inlineWrappers: Set<String> = [
        "AppStorage", "Environment", "EnvironmentObject", "FetchRequest", "GestureState", "Namespace",
        "Query", "ScaledMetric", "SceneStorage", "State", "StateObject",
    ]

    static let ignoredModifiers: [String: String] = [
        "previewContext":
            "previewContext is ignored in a #Preview macro. Use the macro initializer that matches the context of the desired environment; e.g. Widgets, Live Activities",
        "previewDevice":
            "previewDevice is ignored in a #Preview macro. Use the device picker at the bottom of the Canvas to change which device type is used for the preview.",
        "previewDisplayName":
            "previewDisplayName is ignored in a #Preview macro. Provide the name to the macro initializer, e.g. `#Preview(\"preview name\")`",
        "previewInterfaceOrientation":
            "previewInterfaceOrientation is ignored in a #Preview macro. Use the `traits` argument of the macro to provide the desired orientation, e.g. `.landscapeLeft`.",
        // Xcode's text, unbalanced backtick and all.
        "previewLayout":
            "previewLayout is ignored in a #Preview macro. Use the `traits` argument of the macro to provide the desired layout, e.g. `.sizeThatFitsLayout.",
    ]

    var warnings: [Diagnostic] = []

    static func find(in node: some SyntaxProtocol) -> [Diagnostic] {
        let visitor = PreviewWarnings(viewMode: .sourceAccurate)
        visitor.walk(node)
        return visitor.warnings
    }

    func warn(_ node: some SyntaxProtocol, _ message: String) {
        warnings.append(Diagnostic(node: node, message: PreviewDiagnostic(message: message, severity: .warning)))
    }

    override func visit(_ node: VariableDeclSyntax) -> SyntaxVisitorContinueKind {
        guard node.parent?.is(CodeBlockItemSyntax.self) == true,
            previewableAttribute(node.attributes) == nil
        else {
            return .visitChildren
        }
        for element in node.attributes {
            guard let attribute = element.as(AttributeSyntax.self) else { continue }
            let name = attribute.attributeName.trimmedDescription.split(separator: ".").last.map(String.init) ?? ""
            if Self.inlineWrappers.contains(name) {
                warn(attribute, "'@\(name)' used inline will not work unless tagged with '@Previewable'")
            }
        }
        return .visitChildren
    }

    override func visit(_ node: MemberAccessExprSyntax) -> SyntaxVisitorContinueKind {
        if let message = Self.ignoredModifiers[node.declName.baseName.text] {
            warn(node.declName, message)
        }
        return .visitChildren
    }
}

/// Finds an `@Previewable` below a preview body's top level.
final class NestedPreviewable: SyntaxVisitor {
    var found: AttributeSyntax?

    static func first(in node: some SyntaxProtocol) -> AttributeSyntax? {
        let visitor = NestedPreviewable(viewMode: .sourceAccurate)
        visitor.walk(node)
        return visitor.found
    }

    override func visit(_ node: AttributeSyntax) -> SyntaxVisitorContinueKind {
        if found == nil, node.attributeName.trimmedDescription == "Previewable" {
            found = node
        }
        return .skipChildren
    }
}
