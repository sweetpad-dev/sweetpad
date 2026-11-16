import FoundationModels

@Generable(
    name: "Compass direction",
    description: "A direction"
)
enum NamedDirection {
    case north
    case south
}

@Generable(
    name: "Shape choice",
    description: "A shape",
    representNilExplicitlyInGeneratedContent: true
)
enum NamedShape {
    case none
    case circle(radius: Double?)
}
