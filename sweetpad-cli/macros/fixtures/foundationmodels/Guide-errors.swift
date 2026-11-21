import FoundationModels

struct NotGenerable {
    @Guide(description: "No effect")
    var value: String
}

@Generable
struct BadGuide {
    @Guide(description: "Computed")
    var computed: String { "x" }
}

