# Localization extraction captures

These sources are synthetic. The expected UTF-16 files come from Xcode's
`extractLocStrings` with the custom `WMFLocalizedString` prefix:

```sh
xcrun extractLocStrings -s WMFLocalizedString -o expected Strings.swift Strings.m
```

The capture includes Swift labels, Objective-C calls, duplicate comments,
tables, escaped text, dynamic keys, and positional printf conversion. A single
placeholder keeps its original format. Unsupported command options fail
explicitly. Literal extraction does not evaluate source expressions.
