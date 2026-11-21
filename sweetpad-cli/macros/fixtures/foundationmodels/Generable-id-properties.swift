import FoundationModels

@Generable
struct StoredID {
    var id: String
    var value: Int
}

@Generable
struct ImmutableStoredID {
    let id: String
    var value: Int
}

@Generable
struct ComputedID {
    var id: String { "id" }
    var value: Int
}
