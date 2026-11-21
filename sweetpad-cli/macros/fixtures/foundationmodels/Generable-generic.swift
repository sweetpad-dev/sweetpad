import FoundationModels

@Generable
struct Box<Value: Generable> {
    var value: Value
    var values: [Value]
}

