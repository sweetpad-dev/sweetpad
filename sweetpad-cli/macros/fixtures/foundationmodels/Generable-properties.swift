import FoundationModels

@Generable
struct Properties {
    let immutable: String
    var defaulted: Int = 42
    static var shared: Bool = true
    var computed: String { immutable }
    var observed: Double = 0 { didSet {} }
}

