import FoundationModels

@Generable
struct CustomPartial {
    var value: String

    struct PartiallyGenerated: ConvertibleFromGeneratedContent {
        init(_ content: GeneratedContent) throws {}
    }
}

@Generable
enum CustomEnumPartial {
    case value(String)

    enum PartiallyGenerated: ConvertibleFromGeneratedContent {
        case value(String.PartiallyGenerated?)

        init(_ content: GeneratedContent) throws {
            self = .value(try content.value(forProperty: "value"))
        }
    }
}
