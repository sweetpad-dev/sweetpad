import OpenAppleMacrosBase

public var foundationModelsMacros: [Macro.Type] {
    [
        GenerableMacro.self,
        GuideMacro.self,
        SessionPropertyEntryMacro.self,
        SessionPropertyEntryDefaultValueMacro.self,
    ]
}

