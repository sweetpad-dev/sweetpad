import FoundationModels

// A generable type inside another, with a guide that takes a description and
// a count, as Ice Cubes' hashtag assistant declares it.
@available(macOS 27.0, *)
public struct Assistant {
    @Generable
    struct Tags {
        @Guide(description: "The value of the hashtags, must be camelCased and prefixed with a # symbol.", .count(5))
        let values: [String]
    }
}
