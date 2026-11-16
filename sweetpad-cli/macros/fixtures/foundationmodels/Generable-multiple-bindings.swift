import FoundationModels

@Generable
struct MultipleBindings {
    var first: String, second: String
    var inferred = 1, explicit: Int
}
