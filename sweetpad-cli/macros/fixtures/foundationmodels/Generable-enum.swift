import FoundationModels

@Generable(description: "A color")
enum Color {
    case red
    case green
    case rgb(red: Int, green: Int, blue: Int)
}

