import FoundationModels

@available(macOS 27.0, *)
extension SessionPropertyValues {
    @SessionPropertyEntry let immutable: Int = 0
    @SessionPropertyEntry var missingType
    @SessionPropertyEntry var missingDefault: Int
    @SessionPropertyEntry static var shared: Int = 0
    @SessionPropertyEntry var computed: Int { 0 }
}

