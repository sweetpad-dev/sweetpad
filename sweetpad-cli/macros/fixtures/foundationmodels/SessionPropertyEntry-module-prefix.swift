import FoundationModels

enum Other {
    struct SessionPropertyValues {}
}

@available(macOS 27.0, *)
extension Other.SessionPropertyValues {
    @SessionPropertyEntry var value: Int = 0
}
