// 
// GeneratedStringSymbols_Named.swift
// Auto-Generated symbols for localized strings defined in “Named.xcstrings”.
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
    /// Namespace for strings in file “Named.xcstrings”.
    enum Named {
        /**
         Localized string for key “ExplicitFirst” in table “Named.xcstrings”.
         */
        static func explicitFirst(_ arg1: Int, _ arg2: String) -> LocalizedStringResource {
            LocalizedStringResource("ExplicitFirst", defaultValue: "\(arg1, specifier: "%lld")\(arg2)", table: "Named", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “ExplicitSecond” in table “Named.xcstrings”.
         */
        static func explicitSecond(_ arg1: String, _ arg2: Int) -> LocalizedStringResource {
            LocalizedStringResource("ExplicitSecond", defaultValue: "\(arg1)\(arg2, specifier: "%lld")", table: "Named", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “ExplicitSingle” in table “Named.xcstrings”.
         */
        static func explicitSingle(_ arg1: Int) -> LocalizedStringResource {
            LocalizedStringResource("ExplicitSingle", defaultValue: "\(arg1, specifier: "%lld")", table: "Named", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “NamedCount” in table “Named.xcstrings”.
         */
        static func namedCount(count: Int) -> LocalizedStringResource {
            LocalizedStringResource("NamedCount", defaultValue: "\(count, specifier: "%lld")", table: "Named", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “NamedMix” in table “Named.xcstrings”.
         */
        static func namedMix(name: String, count: Int) -> LocalizedStringResource {
            LocalizedStringResource("NamedMix", defaultValue: "\(name)\(count, specifier: "%lld")", table: "Named", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “NamedString” in table “Named.xcstrings”.
         */
        static func namedString(name: String) -> LocalizedStringResource {
            LocalizedStringResource("NamedString", defaultValue: "\(name)", table: "Named", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “NewManual” in table “Named.xcstrings”.
         */
        static var newManual: LocalizedStringResource {
            LocalizedStringResource("NewManual", table: "Named", bundle: resourceBundleDescription)
        }
    }
}