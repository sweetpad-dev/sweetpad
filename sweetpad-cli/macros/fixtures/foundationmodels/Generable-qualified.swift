import FoundationModels

@FoundationModels.Generable(description: "Qualified")
struct Qualified {
    @FoundationModels.Guide(description: "A value", .minimum(0))
    var value: Swift.Int
}

