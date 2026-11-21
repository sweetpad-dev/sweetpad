import OpenAppleMacrosBase
import SwiftDiagnostics

struct GenerableMacro: MemberMacro, ExtensionMacro {
    static func expansion(
        of node: AttributeSyntax,
        providingMembersOf declaration: some DeclGroupSyntax,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [DeclSyntax] {
        if explicitlyConformsToGenerable(declaration) {
            context.diagnose(Diagnostic(
                node: node,
                message: FoundationModelsDiagnostic("Type already conforms to 'Generable'. Remove the '@Generable' macro or delete the existing conformance.")
            ))
            return []
        }
        let options = GenerableOptions(node)
        if let structure = declaration.as(StructDeclSyntax.self) {
            return structMembers(structure, options: options)
        }
        if let enumeration = declaration.as(EnumDeclSyntax.self) {
            return enumMembers(enumeration, options: options, node: node, in: context)
        }
        context.diagnose(Diagnostic(
            node: node,
            message: FoundationModelsDiagnostic("'@Generable' can only be used on structs and enums.")
        ))
        return []
    }

    static func expansion(
        of node: AttributeSyntax,
        attachedTo declaration: some DeclGroupSyntax,
        providingExtensionsOf type: some TypeSyntaxProtocol,
        conformingTo protocols: [TypeSyntax],
        in context: some MacroExpansionContext
    ) throws -> [ExtensionDeclSyntax] {
        guard !explicitlyConformsToGenerable(declaration) else { return [] }
        let access = accessPrefix(of: declaration)
        let attributes = availabilityPrefix(of: declaration)
        if let structure = declaration.as(StructDeclSyntax.self) {
            let properties = storedProperties(of: structure)
            let assignments = renderConditional(properties) {
                "self.\($0.name.text) = try content.value(forProperty: \(swiftStringLiteral($0.keyName)))"
            }
            let initializerBody = assignments.isEmpty ? "" : "\n\(indent(assignments, by: 4))\n"
            let source = """
            \(attributes)extension \(type.trimmed): nonisolated FoundationModels.Generable {
            \(indent("nonisolated \(access)init(_ content: FoundationModels.GeneratedContent) throws {\(initializerBody)}", by: 4))
            }
            """
            return [
                try ExtensionDeclSyntax("\(raw: source)")
            ]
        }
        if let enumeration = declaration.as(EnumDeclSyntax.self) {
            let cases = enumCases(of: enumeration)
            let flatCases = flatElements(cases)
            guard !flatCases.isEmpty else { return [] }
            let body: String
            if flatCases.contains(where: { !$0.values.isEmpty }) {
                body = enumContentInitializer(cases: cases, partial: false)
            } else if isRawValueEnum(enumeration) {
                body = rawEnumInitializer()
            } else {
                body = plainEnumInitializer(cases: cases)
            }
            let source = """
            \(attributes)extension \(type.trimmed): nonisolated FoundationModels.Generable {
            \(indent("nonisolated \(access)init(_ content: FoundationModels.GeneratedContent) throws {\n\(indent(body, by: 4))\n}", by: 4))
            }
            """
            return [try ExtensionDeclSyntax("\(raw: source)")]
        }
        return []
    }
}

private struct GenerableOptions {
    var name: ExprSyntax?
    var description: ExprSyntax?
    var explicitNil: ExprSyntax?

    init(_ attribute: AttributeSyntax) {
        guard case .argumentList(let arguments) = attribute.arguments else { return }
        for argument in arguments {
            switch argument.label?.text {
            case "name": name = argument.expression
            case "description": description = argument.expression
            case "representNilExplicitlyInGeneratedContent": explicitNil = argument.expression
            default: break
            }
        }
    }
}

private struct StoredProperty {
    var name: TokenSyntax
    var keyName: String
    var type: TypeSyntax
    var guide: GuideOptions?
}

private struct GuideOptions {
    var description: ExprSyntax?
    var guides: [ExprSyntax]
}

private enum ConditionalElement<Element> {
    case element(Element)
    case ifConfig([ConditionalClause<Element>])
}

private struct ConditionalClause<Element> {
    var poundKeyword: String
    var condition: String?
    var elements: [ConditionalElement<Element>]
}

private func conditionalElements<Element>(
    in members: MemberBlockItemListSyntax,
    transform: (DeclSyntax) -> [Element]
) -> [ConditionalElement<Element>] {
    members.flatMap { member -> [ConditionalElement<Element>] in
        if let ifConfig = member.decl.as(IfConfigDeclSyntax.self) {
            let clauses = ifConfig.clauses.map { clause -> ConditionalClause<Element> in
                let declarations = clause.elements?.as(MemberBlockItemListSyntax.self)
                return ConditionalClause(
                    poundKeyword: clause.poundKeyword.trimmed.description,
                    condition: clause.condition?.trimmed.description,
                    elements: declarations.map {
                        conditionalElements(in: $0, transform: transform)
                    } ?? []
                )
            }
            return [.ifConfig(clauses)]
        }
        return transform(member.decl).map(ConditionalElement.element)
    }
}

private func flatElements<Element>(_ elements: [ConditionalElement<Element>]) -> [Element] {
    elements.flatMap { element in
        switch element {
        case .element(let value):
            return [value]
        case .ifConfig(let clauses):
            return clauses.flatMap { flatElements($0.elements) }
        }
    }
}

private func containsIfConfig<Element>(_ elements: [ConditionalElement<Element>]) -> Bool {
    elements.contains {
        if case .ifConfig = $0 { return true }
        return false
    }
}

private func renderConditional<Element>(
    _ elements: [ConditionalElement<Element>],
    transform: (Element) -> String
) -> String {
    elements.map { element in
        switch element {
        case .element(let value):
            return transform(value)
        case .ifConfig(let clauses):
            let body = clauses.map { clause in
                let directive = clause.condition.map {
                    "\(clause.poundKeyword) \($0)"
                } ?? clause.poundKeyword
                let elements = renderConditional(clause.elements, transform: transform)
                return elements.isEmpty ? directive : "\(directive)\n\(elements)"
            }.joined(separator: "\n")
            return "\(body)\n#endif"
        }
    }.joined(separator: "\n")
}

private func conditionalDeclarations<Element>(
    _ elements: [ConditionalElement<Element>],
    transform: (Element) -> DeclSyntax
) -> [DeclSyntax] {
    elements.map { element in
        switch element {
        case .element(let value):
            return transform(value)
        case .ifConfig:
            return parseDeclaration(renderConditional([element]) {
                transform($0).trimmed.description
            })
        }
    }
}

private func renderConditionalWithBlankAfterIfConfig<Element>(
    _ elements: [ConditionalElement<Element>],
    transform: (Element) -> String
) -> String {
    var result = ""
    for (index, element) in elements.enumerated() {
        if index > 0 {
            if case .ifConfig = elements[index - 1] {
                result += "\n\n"
            } else {
                result += "\n"
            }
        }
        result += renderConditional([element], transform: transform)
    }
    return result
}

private func storedProperties(of declaration: some DeclGroupSyntax) -> [ConditionalElement<StoredProperty>] {
    conditionalElements(in: declaration.memberBlock.members) { member -> [StoredProperty] in
        guard let variable = member.as(VariableDeclSyntax.self),
              !variable.modifiers.contains(where: {
                  $0.name.tokenKind == .keyword(.static) || $0.name.tokenKind == .keyword(.class)
              }) else { return [] }
        return variable.bindings.compactMap { binding in
            guard binding.accessorBlock == nil,
                  let name = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier,
                  let type = binding.typeAnnotation?.type else { return nil }
            return StoredProperty(
                name: name.trimmed,
                keyName: unbackticked(name.text),
                type: type.trimmed,
                guide: guideOptions(in: variable.attributes)
            )
        }
    }
}

private func guideOptions(in attributes: AttributeListSyntax) -> GuideOptions? {
    guard let attribute = attributes.compactMap({ $0.as(AttributeSyntax.self) }).first(where: {
        $0.attributeName.trimmed.description == "Guide"
    }) else { return nil }
    var result = GuideOptions(description: nil, guides: [])
    guard case .argumentList(let arguments) = attribute.arguments else { return result }
    for argument in arguments {
        if argument.label?.text == "description" {
            result.description = argument.expression
        } else {
            result.guides.append(argument.expression)
        }
    }
    return result
}

private func structMembers(_ declaration: StructDeclSyntax, options: GenerableOptions) -> [DeclSyntax] {
    let properties = storedProperties(of: declaration)
    let access = accessPrefix(of: declaration)
    let schema: DeclSyntax
    if containsIfConfig(properties) {
        schema = conditionalSchemaDeclaration(
            access: access,
            options: options,
            properties: properties,
            typeExpression: options.name == nil ? "Self.self" : nil
        )
    } else {
        schema = schemaDeclaration(
            access: access,
            options: options,
            properties: flatElements(properties),
            typeExpression: options.name == nil ? "Self.self" : nil
        )
    }
    let generatedContent = generatedContentDeclaration(access: access, properties: properties, explicitNil: options.explicitNil)
    let explicitlyIdentifiable = declaration.inheritanceClause?.inheritedTypes.contains(where: {
        $0.type.trimmed.description.split(separator: ".").last == "Identifiable"
    }) ?? false
    if declaresPartiallyGenerated(in: declaration.memberBlock.members) {
        return [schema, generatedContent]
    }
    let partial = partialStructDeclaration(
        access: access,
        properties: properties,
        synthesizeID: !explicitlyIdentifiable
            && !flatElements(properties).contains(where: { $0.keyName == "id" })
    )
    return [schema, generatedContent, partial]
}

private func declaresPartiallyGenerated(in members: MemberBlockItemListSyntax) -> Bool {
    members.contains { member in
        let declaration = member.decl
        if declaration.as(StructDeclSyntax.self)?.name.text == "PartiallyGenerated"
            || declaration.as(EnumDeclSyntax.self)?.name.text == "PartiallyGenerated"
            || declaration.as(ClassDeclSyntax.self)?.name.text == "PartiallyGenerated"
            || declaration.as(ActorDeclSyntax.self)?.name.text == "PartiallyGenerated"
            || declaration.as(TypeAliasDeclSyntax.self)?.name.text == "PartiallyGenerated" {
            return true
        }
        if let ifConfig = declaration.as(IfConfigDeclSyntax.self) {
            return ifConfig.clauses.contains { clause in
                clause.elements?.as(MemberBlockItemListSyntax.self).map(declaresPartiallyGenerated(in:)) ?? false
            }
        }
        return false
    }
}

private func schemaDeclaration(
    access: String,
    options: GenerableOptions,
    properties: [StoredProperty],
    typeExpression: String?
) -> DeclSyntax {
    var arguments: [String] = []
    if let name = options.name {
        arguments.append("name: \(name.trimmed)")
    } else if let typeExpression {
        arguments.append("type: \(typeExpression)")
    }
    if let description = options.description {
        arguments.append("description: \(description.trimmed)")
    }
    if let explicitNil = options.explicitNil {
        arguments.append("representNilExplicitlyInGeneratedContent: \(explicitNil.trimmed)")
    }
    let propertyLines = properties.map(propertySchema).joined(separator: ",\n")
    arguments.append("properties: [\n\(indent(propertyLines, by: 4))\n]")
    let argumentsText = arguments.map { indent($0, by: 8) }.joined(separator: ",\n")
    return """
    nonisolated \(raw: access)static var generationSchema: FoundationModels.GenerationSchema {
        FoundationModels.GenerationSchema(
    \(raw: argumentsText)
        )
    }
    """
}

private func conditionalSchemaDeclaration(
    access: String,
    options: GenerableOptions,
    properties: [ConditionalElement<StoredProperty>],
    typeExpression: String?
) -> DeclSyntax {
    let additions = renderConditional(properties) {
        "properties.append(\(propertySchema($0)))"
    }
    var arguments: [String] = []
    if let name = options.name {
        arguments.append("name: \(name.trimmed)")
    } else if let typeExpression {
        arguments.append("type: \(typeExpression)")
    }
    if let description = options.description {
        arguments.append("description: \(description.trimmed)")
    }
    if let explicitNil = options.explicitNil {
        arguments.append("representNilExplicitlyInGeneratedContent: \(explicitNil.trimmed)")
    }
    arguments.append("properties: properties")
    let argumentsText = arguments.map { indent($0, by: 8) }.joined(separator: ",\n")
    return parseDeclaration(
        "nonisolated \(access)static var generationSchema: FoundationModels.GenerationSchema {\n" +
        "    var properties = [FoundationModels.GenerationSchema.Property]()\n" +
        indent(additions, by: 4) + "\n" +
        "    return FoundationModels.GenerationSchema(\n" + argumentsText + "\n    )\n" +
        "}"
    )
}

private func propertySchema(_ property: StoredProperty) -> String {
    var arguments = ["name: \(swiftStringLiteral(property.keyName))"]
    if let description = property.guide?.description {
        arguments.append("description: \(description.trimmed)")
    }
    arguments.append("type: \(property.type.trimmed).self")
    if let guides = property.guide?.guides, !guides.isEmpty {
        arguments.append("guides: [\(guides.map { $0.trimmed.description }.joined(separator: ", "))]")
    }
    return "FoundationModels.GenerationSchema.Property(\(arguments.joined(separator: ", ")))"
}

private func generatedContentDeclaration(
    access: String,
    properties: [ConditionalElement<StoredProperty>],
    explicitNil: ExprSyntax?
) -> DeclSyntax {
    let additions = renderConditional(properties) {
        "addProperty(name: \(swiftStringLiteral($0.keyName)), value: self.\($0.name.text))"
    }
    let explicitNilExpression = explicitNil?.trimmed.description ?? "false"
    var lines = [
        "nonisolated \(access)var generatedContent: GeneratedContent {",
        "    let explicitNil = \(explicitNilExpression)",
        "    var properties = [(name: String, value: any ConvertibleToGeneratedContent)]()",
    ]
    if !additions.isEmpty {
        lines.append(indent(additions, by: 4))
    }
    lines += [
        "    return GeneratedContent(",
        "        properties: properties,",
        "        uniquingKeysWith: { _, second in",
        "            second",
        "        }",
        "    )",
        "    func addProperty(name: String, value: some Generable) {",
        "      properties.append((name: name, value: value))",
        "    }",
        "    func addProperty(name: String, value: (some Generable)?) {",
        "      if explicitNil || value != nil {",
        "        properties.append((name: name, value: value))",
        "      }",
        "    }",
        "}",
    ]
    return parseDeclaration(lines.joined(separator: "\n"))
}

private func partialStructDeclaration(
    access: String,
    properties: [ConditionalElement<StoredProperty>],
    synthesizeID: Bool
) -> DeclSyntax {
    let conformance = synthesizeID
        ? "Identifiable, nonisolated FoundationModels.ConvertibleFromGeneratedContent"
        : "nonisolated FoundationModels.ConvertibleFromGeneratedContent"
    var declarations: [String] = []
    var assignments: [String] = []
    if synthesizeID {
        declarations.append("\(access)var id: GenerationID")
        assignments.append("self.id = content.id ?? GenerationID()")
    }
    let propertyDeclarations = renderConditional(properties) {
        "\(access)var \($0.name.text): \($0.type.trimmed).PartiallyGenerated?"
    }
    let propertyAssignments = renderConditional(properties) {
        "self.\($0.name.text) = try content.value(forProperty: \(swiftStringLiteral($0.keyName)))"
    }
    if !propertyDeclarations.isEmpty { declarations.append(propertyDeclarations) }
    if !propertyAssignments.isEmpty { assignments.append(propertyAssignments) }
    return """
    nonisolated \(raw: access)struct PartiallyGenerated: \(raw: conformance) {
        \(raw: declarations.joined(separator: "\n"))
        nonisolated \(raw: access)init(_ content: FoundationModels.GeneratedContent) throws {
            \(raw: assignments.joined(separator: "\n"))
        }
    }
    """
}

private struct EnumCaseInfo {
    var name: TokenSyntax
    var rawValue: ExprSyntax?
    var values: [EnumValueInfo]
}

private struct EnumValueInfo {
    var label: TokenSyntax?
    var propertyName: String
    var keyName: String
    var type: TypeSyntax
}

private func enumCases(of declaration: EnumDeclSyntax) -> [ConditionalElement<EnumCaseInfo>] {
    conditionalElements(in: declaration.memberBlock.members) { member -> [EnumCaseInfo] in
        guard let caseDecl = member.as(EnumCaseDeclSyntax.self) else { return [] }
        return caseDecl.elements.map { element in
            let parameters = Array(element.parameterClause?.parameters ?? [])
            var nextUnlabeled = 0
            let values = parameters.map { parameter -> EnumValueInfo in
                let label = parameter.firstName?.tokenKind == .wildcard ? nil : parameter.firstName
                let propertyName: String
                if let internalName = parameter.secondName {
                    propertyName = internalName.text
                } else if let label {
                    propertyName = label.text
                } else if nextUnlabeled == 0 {
                    propertyName = "value"
                    nextUnlabeled += 1
                } else {
                    propertyName = "value\(nextUnlabeled)"
                    nextUnlabeled += 1
                }
                return EnumValueInfo(
                    label: label,
                    propertyName: propertyName,
                    keyName: unbackticked(propertyName),
                    type: parameter.type.trimmed
                )
            }
            return EnumCaseInfo(name: element.name.trimmed, rawValue: element.rawValue?.value, values: values)
        }
    }
}

private func enumMembers(
    _ declaration: EnumDeclSyntax,
    options: GenerableOptions,
    node: AttributeSyntax,
    in context: some MacroExpansionContext
) -> [DeclSyntax] {
    let cases = enumCases(of: declaration)
    let flatCases = flatElements(cases)
    guard !flatCases.isEmpty else {
        context.diagnose(Diagnostic(
            node: node,
            message: FoundationModelsDiagnostic("Generable enums must have at least one case.")
        ))
        return []
    }
    let access = accessPrefix(of: declaration)
    if flatCases.contains(where: { !$0.values.isEmpty }) {
        return payloadEnumMembers(
            cases: cases,
            access: access,
            options: options,
            includePartial: !declaresPartiallyGenerated(in: declaration.memberBlock.members)
        )
    }
    return plainEnumMembers(declaration, cases: cases, access: access, options: options)
}

private func plainEnumMembers(
    _ declaration: EnumDeclSyntax,
    cases: [ConditionalElement<EnumCaseInfo>],
    access: String,
    options: GenerableOptions
) -> [DeclSyntax] {
    let raw = isRawValueEnum(declaration)
    let flatCases = flatElements(cases)
    let values = flatCases.map {
        raw ? "\($0.name.text).rawValue" : swiftStringLiteral($0.name.text)
    }.joined(separator: ", ")
    var arguments = [options.name.map { "name: \($0.trimmed)" } ?? "type: Self.self"]
    if let description = options.description {
        arguments.append("description: \(description.trimmed)")
    }
    let schema: DeclSyntax
    if containsIfConfig(cases) {
        let additions = renderConditional(cases) {
            let value = raw ? "\($0.name.text).rawValue" : swiftStringLiteral($0.name.text)
            return "properties.append(\(value))"
        }
        arguments.append("anyOf: properties")
        schema = parseDeclaration(
            "nonisolated \(access)static var generationSchema: FoundationModels.GenerationSchema {\n" +
            "    var properties = [String]()\n" + indent(additions, by: 4) + "\n" +
            "    return FoundationModels.GenerationSchema(\(arguments.joined(separator: ", ")))\n" +
            "}"
        )
    } else {
        arguments.append("anyOf: [\(values)]")
        schema = parseDeclaration(
            "nonisolated \(access)static var generationSchema: FoundationModels.GenerationSchema {\n" +
            "    FoundationModels.GenerationSchema(\(arguments.joined(separator: ", ")))\n" +
            "}"
        )
    }
    let content: DeclSyntax
    if raw {
        content = parseDeclaration(
            "nonisolated \(access)var generatedContent: GeneratedContent {\n" +
            "    rawValue.generatedContent\n" +
            "}"
        )
    } else {
        let bodyIndent = containsIfConfig(cases) ? 0 : 4
        let branches = renderConditionalWithBlankAfterIfConfig(cases) {
            "case .\($0.name.text):\n" + indent(
                "\(swiftStringLiteral($0.name.text)).generatedContent",
                by: bodyIndent
            )
        }
        content = parseDeclaration(
            "nonisolated \(access)var generatedContent: GeneratedContent {\n" +
            "    switch self {\n" + indent(branches, by: 4) + "\n    }\n" +
            "}"
        )
    }
    return [schema, content]
}

private func payloadEnumMembers(
    cases: [ConditionalElement<EnumCaseInfo>],
    access: String,
    options: GenerableOptions,
    includePartial: Bool
) -> [DeclSyntax] {
    let partialCases = renderConditional(cases) { enumCaseDeclaration($0, partial: true) }
    let partialInit = enumContentInitializer(cases: cases, partial: true)
    let partial = parseDeclaration(
        "nonisolated \(access)enum PartiallyGenerated: nonisolated FoundationModels.ConvertibleFromGeneratedContent {\n" +
        indent(partialCases, by: 4) + "\n" +
        "    nonisolated \(access)init(_ content: FoundationModels.GeneratedContent) throws {\n" +
        indent(partialInit, by: 8) + "\n" +
        "    }\n" +
        "}"
    )
    var schemaArguments = [options.name.map { "name: \($0.trimmed)" } ?? "type: Self.self"]
    if let description = options.description {
        schemaArguments.append("description: \(description.trimmed)")
    }
    let schema: DeclSyntax
    if containsIfConfig(cases) {
        let additions = renderConditional(cases) {
            "properties.append(Discriminated\(uppercasingFirst($0.name.text)).self)"
        }
        schemaArguments.append("anyOf: properties")
        schema = parseDeclaration(
            "nonisolated \(access)static var generationSchema: FoundationModels.GenerationSchema {\n" +
            "    var properties = [any Generable.Type]()\n" + indent(additions, by: 4) + "\n" +
            "    return FoundationModels.GenerationSchema(\(schemaArguments.joined(separator: ", ")))\n" +
            "}"
        )
    } else {
        let types = flatElements(cases).map {
            "Discriminated\(uppercasingFirst($0.name.text)).self"
        }.joined(separator: ",\n")
        schemaArguments.append("anyOf: [\n\(indent(types, by: 4))\n]")
        let schemaText = schemaArguments.map { indent($0, by: 8) }.joined(separator: ",\n")
        schema = parseDeclaration(
            "nonisolated \(access)static var generationSchema: FoundationModels.GenerationSchema {\n" +
            "    FoundationModels.GenerationSchema(\n" + schemaText + "\n    )\n" +
            "}"
        )
    }
    let discriminators = conditionalDeclarations(cases) { discriminatorDeclaration($0, options: options) }
    let content = enumGeneratedContent(cases: cases, access: access)
    return (includePartial ? [partial] : []) + [schema] + discriminators + [content]
}

private func discriminatorDeclaration(_ enumCase: EnumCaseInfo, options: GenerableOptions) -> DeclSyntax {
    let name = "Discriminated\(uppercasingFirst(enumCase.name.text))"
    let isBacktickedCase = enumCase.name.text.first == "`" && enumCase.name.text.last == "`"
    let declarationName = isBacktickedCase ? "Discriminated" : name
    let conformance = isBacktickedCase ? "" : ": nonisolated FoundationModels.Generable"
    var properties = [StoredProperty(name: .identifier("type"), keyName: "type", type: "String", guide: GuideOptions(
        description: nil,
        guides: [ExprSyntax(".constant(\(raw: swiftStringLiteral(enumCase.name.text)))")]
    ))]
    properties += enumCase.values.map {
        StoredProperty(name: .identifier($0.propertyName), keyName: $0.keyName, type: $0.type, guide: nil)
    }
    let declarations = properties.map { property -> String in
        if property.name.text == "type" {
            return "@Guide(.constant(\(swiftStringLiteral(enumCase.name.text))))\nlet type: String"
        }
        return "let \(property.name.text): \(property.type.trimmed)"
    }.joined(separator: "\n")
    let assignments = properties.map {
        "self.\($0.name.text) = try content.value(forProperty: \(swiftStringLiteral($0.keyName)))"
    }.joined(separator: "\n")
    let schema = schemaDeclaration(access: "", options: GenerableOptionsForDiscriminator(options), properties: properties, typeExpression: "Self.self")
    let generated = generatedContentDeclaration(
        access: "",
        properties: properties.map(ConditionalElement.element),
        explicitNil: options.explicitNil
    )
    let initializer = "nonisolated init(_ content: FoundationModels.GeneratedContent) throws {\n\(indent(assignments, by: 4))\n}"
    let source = "private nonisolated struct \(declarationName)\(conformance) {\n" +
        indent(declarations, by: 4) + "\n" +
        indent(initializer, by: 4) + "\n" +
        indent(schema.trimmed.description, by: 4) + "\n" +
        indent(generated.trimmed.description, by: 4) + "\n" +
        "}"
    return parseDeclaration(source)
}

private func GenerableOptionsForDiscriminator(_ options: GenerableOptions) -> GenerableOptions {
    var result = options
    result.name = nil
    return result
}

private func enumCaseDeclaration(_ enumCase: EnumCaseInfo, partial: Bool) -> String {
    guard !enumCase.values.isEmpty else { return "case \(enumCase.name.text)" }
    let parameters = enumCase.values.map { value in
        let type = "\(value.type.trimmed)\(partial ? ".PartiallyGenerated?" : "")"
        if let label = value.label {
            return "\(label.text): \(type)"
        }
        return type
    }.joined(separator: ", ")
    return "case \(enumCase.name.text)(\(parameters))"
}

private func enumContentInitializer(cases: [ConditionalElement<EnumCaseInfo>], partial: Bool) -> String {
    let branches = renderConditional(cases) { enumCase -> String in
        let name = enumCase.name.text
        guard !enumCase.values.isEmpty else {
            return "case \(swiftStringLiteral(name)):\n    self = .\(name)"
        }
        let arguments = enumCase.values.map { value in
            let expression = "try content.value(forProperty: \(swiftStringLiteral(value.keyName)))"
            return value.label.map { "\($0.text): \(expression)" } ?? expression
        }
        let assignment: String
        if arguments.count == 1 {
            assignment = "self = .\(name)(\(arguments[0]))"
        } else {
            assignment = "self = .\(name)(\n\(indent(arguments.joined(separator: ",\n"), by: 4))\n)"
        }
        return "case \(swiftStringLiteral(name)):\n" + indent(assignment, by: 4)
    }
    return "let type: String = try content.value(forProperty: \"type\")\n" +
        "switch type {\n" + branches + "\n" +
        "default:\n" + indent(unexpectedValueThrow(variable: "type", adjective: "type"), by: 4) + "\n" +
        "}"
}

private func enumGeneratedContent(cases: [ConditionalElement<EnumCaseInfo>], access: String) -> DeclSyntax {
    let bodyIndent = containsIfConfig(cases) ? 0 : 4
    let branches = renderConditional(cases) { enumCase -> String in
        let bindings = enumCase.values.map { "let \($0.propertyName)" }.joined(separator: ", ")
        let pattern = ".\(enumCase.name.text)\(bindings.isEmpty ? "" : "(\(bindings))")"
        let entries = (["\"type\": \(swiftStringLiteral(enumCase.name.text))"] + enumCase.values.map {
            "\(swiftStringLiteral($0.keyName)): \($0.propertyName)"
        }).joined(separator: ",\n")
        let bodyEntries = enumCase.values.isEmpty ? entries + ",\n" : entries
        return "case \(pattern):\n" +
            indent(
                "GeneratedContent(\n" +
                "    properties: [\n" + indent(bodyEntries, by: 8) + "\n" +
                "    ]\n" +
                ")",
                by: bodyIndent
            )
    }
    return parseDeclaration(
        "nonisolated \(access)var generatedContent: GeneratedContent {\n" +
        "    switch self {\n" + indent(branches, by: 4) + "\n" +
        "    }\n" +
        "}"
    )
}

private func plainEnumInitializer(cases: [ConditionalElement<EnumCaseInfo>]) -> String {
    let branches = renderConditional(cases) {
        "case \(swiftStringLiteral($0.name.text)):\n    self = .\($0.name.text)"
    }
    return "let rawValue = try content.value(String.self)\n" +
        "switch rawValue {\n" + branches + "\n" +
        "default:\n" + indent(unexpectedValueThrow(variable: "rawValue", adjective: "value"), by: 4) + "\n" +
        "}"
}

private func rawEnumInitializer() -> String {
    "let rawValue = try content.value(String.self)\n" +
        "if let value = Self(rawValue: rawValue) {\n" +
        "    self = value\n" +
        "} else {\n" + indent(unexpectedValueThrow(variable: "rawValue", adjective: "rawValue"), by: 4) + "\n" +
        "}"
}

private func unexpectedValueThrow(variable: String, adjective: String) -> String {
    """
    if #available(iOS 27.0, macOS 27.0, visionOS 27.0, *) {
        throw FoundationModels.GeneratedContent.ParsingError(rawContent: content.jsonString, debugDescription: "Unexpected \(adjective) \\\"\\(\(variable))\\\" for \\(Self.self)")
    } else {
        throw FoundationModels.LanguageModelSession.GenerationError.decodingFailure(FoundationModels.LanguageModelSession.GenerationError.Context(debugDescription: "Unexpected \(adjective) \\\"\\(\(variable))\\\" for \\(Self.self)"))
    }
    """
}

private func isRawValueEnum(_ declaration: EnumDeclSyntax) -> Bool {
    guard let first = declaration.inheritanceClause?.inheritedTypes.first?.type.trimmed.description else {
        return false
    }
    let name = first.split(separator: ".").last.map(String.init) ?? first
    return name == "String"
}

private func accessPrefix(of declaration: some DeclGroupSyntax) -> String {
    if declaration.modifiers.contains(where: { $0.name.tokenKind == .keyword(.public) }) {
        return "public "
    }
    if declaration.modifiers.contains(where: { $0.name.tokenKind == .keyword(.package) }) {
        return "package "
    }
    return ""
}

private func explicitlyConformsToGenerable(_ declaration: some DeclGroupSyntax) -> Bool {
    declaration.inheritanceClause?.inheritedTypes.contains(where: {
        $0.type.trimmed.description.split(separator: ".").last == "Generable"
    }) ?? false
}

private func availabilityPrefix(of declaration: some DeclGroupSyntax) -> String {
    declaration.attributes.compactMap { $0.as(AttributeSyntax.self) }.filter {
        $0.attributeName.trimmed.description.split(separator: ".").last == "available"
    }.map { "\($0.trimmed)\n" }.joined()
}

private func swiftStringLiteral(_ value: String) -> String {
    StringLiteralExprSyntax(content: value).description
}

private func uppercasingFirst(_ value: String) -> String {
    guard let first = value.first else { return value }
    return first.uppercased() + value.dropFirst()
}

private func unbackticked(_ value: String) -> String {
    guard value.count >= 2, value.first == "`", value.last == "`" else { return value }
    return String(value.dropFirst().dropLast())
}

private func indent(_ text: String, by spaces: Int) -> String {
    let prefix = String(repeating: " ", count: spaces)
    return text.split(separator: "\n", omittingEmptySubsequences: false).map { prefix + $0 }.joined(separator: "\n")
}

private func parseDeclaration(_ source: String) -> DeclSyntax {
    "\(raw: source)"
}
