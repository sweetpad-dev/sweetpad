import FoundationModels

@Generable
struct Item: Identifiable {
    let id: GenerationID
    let name: String
}

@Generable
struct ImmutableID: Identifiable {
    let id: String
    var name: String
}

