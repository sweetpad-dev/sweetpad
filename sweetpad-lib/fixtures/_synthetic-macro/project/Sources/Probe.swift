import SweetMacro

// `#stringify` is a freestanding expression macro whose implementation
// (`SweetMacroMacros.StringifyMacro`) lives only in the package's `.macro`
// plugin executable — never in the project graph. The editor resolves it only
// if the BSP passes `-load-plugin-executable <plugin>#SweetMacroMacros`, so an
// unresolved `#stringify` here means the macro plugin arg is missing.
enum Probe {
    static let result: (Int, String) = #stringify(1 + 2)
    static var value: Int { result.0 }
    static var source: String { result.1 }
}
