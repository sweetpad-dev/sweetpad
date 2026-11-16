import FoundationModels

@Generable
struct GuideVariants {
    @Guide(description: "Multiple")
    var first: String, second: String

    @Guide(description: "Static")
    static var shared: String = "shared"
}
