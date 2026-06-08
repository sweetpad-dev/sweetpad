/// A macro that produces both a value and a string containing the source code
/// that generated the value. `#stringify(x + y)` expands to `(x + y, "x + y")`.
@freestanding(expression)
public macro stringify<T>(_ value: T) -> (T, String) =
    #externalMacro(module: "SweetMacroMacros", type: "StringifyMacro")
