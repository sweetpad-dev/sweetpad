import FoundationModels

@Generable
struct KeywordProperties {
    var `default`: String
    var `repeat`: Int
}

@Generable
enum KeywordCases {
    case `default`
    case payload(`repeat`: String)
}
