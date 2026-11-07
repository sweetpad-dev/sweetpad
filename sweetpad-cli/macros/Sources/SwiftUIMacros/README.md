# Portable SwiftUI macros

The macro implementations come from
[xtool-org/OpenAppleMacros](https://github.com/xtool-org/OpenAppleMacros)
at revision `b29af51df1f1d6d4168c73955ad247ca1d1866ad` (version 1.4.1).
The MIT license is in `LICENSE.md`.
`OpenAppleMacrosBase` contains the matching upstream helpers.

Local changes export the macro list for sweetpad and permit implicit `nil`
defaults on optional `@Entry` properties.
The source implementation also preserves the upstream State and Animatable macros.
