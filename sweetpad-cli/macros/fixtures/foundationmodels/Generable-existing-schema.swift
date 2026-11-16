import FoundationModels

@Generable
struct ExistingGenerationSchema {
    static var generationSchema: GenerationSchema {
        GenerationSchema(type: Self.self, properties: [])
    }

    var value: String
}

@Generable
struct ExistingSchema {
    static var schema: GenerationSchema {
        GenerationSchema(type: Self.self, properties: [])
    }

    var value: String
}
