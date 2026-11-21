import FoundationModels

@Generable
struct ConditionalBranches {
    #if os(macOS)
    var first: String
    var second: Int
    #elseif os(iOS)
    var mobile: Bool
    #else
    var fallback: Double
    #endif
}

@Generable
enum ConditionalPlain {
    #if os(macOS)
    case mac
    #else
    case other
    #endif

    case common
}

@Generable
enum ConditionalRaw: String {
    #if os(macOS)
    case mac = "darwin"
    #else
    case other = "other"
    #endif

    case common
}
