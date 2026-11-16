import FoundationModels

@Generable
enum Value {
    case none
    case integer(Int)
    case pair(String, count: Int)
}

