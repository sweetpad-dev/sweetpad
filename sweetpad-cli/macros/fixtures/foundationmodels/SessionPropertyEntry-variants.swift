import FoundationModels

@available(macOS 27.0, *)
extension SessionPropertyValues {
    @SessionPropertyEntry var inferred = 42
    @SessionPropertyEntry var optional: String? = nil
}

