import FoundationModels

@available(macOS 27.0, *)
extension SessionPropertyValues {
    @SessionPropertyEntry var first = 1, second = 2
    @SessionPropertyEntry var (left, right) = (1, 2)
}
