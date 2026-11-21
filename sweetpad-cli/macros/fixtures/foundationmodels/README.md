# FoundationModels fixtures

Every fixture but `Generable-nested-in-type.swift` comes from the
`IntegrationTests/FoundationModels` folder of
[xtool-org/OpenAppleMacros](https://github.com/xtool-org/OpenAppleMacros)
at revision `b29af51df1f1d6d4168c73955ad247ca1d1866ad` (MIT, the license in
`Sources/FoundationModelsMacros/LICENSE.md`). They target macOS 27:

    check.py --fixtures foundationmodels --modules FoundationModelsMacros \
        --sdk macosx --target arm64-apple-macos27.0
