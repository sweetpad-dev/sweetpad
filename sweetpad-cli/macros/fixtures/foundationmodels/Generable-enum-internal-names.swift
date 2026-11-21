import FoundationModels

@Generable
enum LabeledValue {
    case one(_ value: String)
    case two(_ first: String, _ second: Int)
}
