import FoundationModels

@available(macOS 27.0, *)
extension SessionPropertyValues {
    @SessionPropertyEntry
    var activatedSkills: [String: Bool] = [:]
}
