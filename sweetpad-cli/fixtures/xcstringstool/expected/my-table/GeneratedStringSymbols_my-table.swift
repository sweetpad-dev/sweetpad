// 
// GeneratedStringSymbols_my-table.swift
// Auto-Generated symbols for localized strings defined in “my-table.xcstrings”.
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
    /// Namespace for strings in file “my-table.xcstrings”.
    enum MyTable {
        /**
         Localized string for key “settings.title” in table “my-table.xcstrings”.
         */
        static var settingsTitle: LocalizedStringResource {
            LocalizedStringResource("settings.title", table: "my-table", bundle: resourceBundleDescription)
        }
    }
}