// 
// GeneratedStringSymbols_Aliases.swift
// Auto-Generated symbols for localized strings defined in “Aliases.xcstrings”.
// 

import Foundation

#if SWIFT_PACKAGE
private nonisolated let resourceBundle = Foundation.Bundle.module
@available(macOS 13, iOS 16, tvOS 16, watchOS 9, *)
private nonisolated let resourceBundleDescription = LocalizedStringResource.BundleDescription.atURL(resourceBundle.bundleURL)
#else

private class ResourceBundleClass {}
@available(macOS 13, iOS 16, tvOS 16, watchOS 9, *)
private nonisolated let resourceBundleDescription = LocalizedStringResource.BundleDescription.forClass(ResourceBundleClass.self)
#endif

@available(macOS 13, iOS 16, tvOS 16, watchOS 9, *)
nonisolated extension LocalizedStringResource {
    /// Namespace for strings in file “Aliases.xcstrings”.
    enum Aliases {
        /**
         Localized string for key “hello” in table “Aliases.xcstrings”.
         */
        static var hello: LocalizedStringResource {
            LocalizedStringResource("hello", table: "Aliases", bundle: resourceBundleDescription)
        }
    }
}