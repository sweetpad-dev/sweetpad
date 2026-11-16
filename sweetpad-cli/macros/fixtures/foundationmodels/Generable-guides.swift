import FoundationModels

@Generable(description: "A person")
struct Person {
    @Guide(description: "Full name")
    var name: String

    @Guide(.range(0...130))
    var age: Int

    @Guide(description: "Nicknames", .minimumCount(1), .maximumCount(3))
    var aliases: [String]
}

