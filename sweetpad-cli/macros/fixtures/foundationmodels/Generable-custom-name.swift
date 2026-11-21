import FoundationModels

@available(macOS 27.0, *)
@Generable(name: "A custom schema", description: "Some description")
struct Payload {
    let value: String
}
