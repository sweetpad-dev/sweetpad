import FoundationMacros
import PreviewsMacros
import SwiftCompilerPlugin
import SwiftDataMacros
import SwiftUIMacros
import TipKitMacros
import SwiftSyntaxMacros

/// The plugin the compiler runs for the modules it's given with
/// `-load-plugin-executable`. It resolves a macro by its module and type
/// name, which is each type's own: `SwiftDataMacros.PersistentModelMacro`.
@main
struct SweetPadMacros: CompilerPlugin {
    let providingMacros: [Macro.Type] = swiftDataMacros + previewsMacros + swiftUIMacros + tipKitMacros + [
        PredicateMacro.self,
        ExpressionMacro.self,
        BundleMacro.self,
    ]
}
