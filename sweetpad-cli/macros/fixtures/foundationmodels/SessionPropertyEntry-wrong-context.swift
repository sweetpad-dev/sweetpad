import FoundationModels

@available(macOS 27.0, *)
struct WrongContext {
    @SessionPropertyEntry var value: Int = 0
}

@available(macOS 27.0, *)
extension FoundationModels.SessionPropertyValues {
    @SessionPropertyEntry var qualified: Bool = false
}

