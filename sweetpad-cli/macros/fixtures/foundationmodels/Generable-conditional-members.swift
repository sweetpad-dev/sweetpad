import FoundationModels

@Generable
struct ConditionalProperties {
    #if os(macOS)
    var platform: String
    #endif

    var value: Int
}

@Generable
enum ConditionalCases {
    #if os(macOS)
    case platform(String)
    #endif

    case fallback
}
