// 
// GeneratedStringSymbols_Probe.swift
// Auto-Generated symbols for localized strings defined in “Probe.xcstrings”.
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
    /// Namespace for strings in file “Probe.xcstrings”.
    enum Probe {
        /**
         Localized string for key “Manual device” in table “Probe.xcstrings”.
         */
        static var manualDevice: LocalizedStringResource {
            LocalizedStringResource("Manual device", table: "Probe", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “Manual device plural %lld” in table “Probe.xcstrings”.
         */
        static func manualDevicePlural(_ arg1: Int) -> LocalizedStringResource {
            LocalizedStringResource("Manual device plural %lld", defaultValue: "\(arg1, specifier: "%lld")", table: "Probe", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “Manual key” in table “Probe.xcstrings”.
         */
        static var manualKey: LocalizedStringResource {
            LocalizedStringResource("Manual key", table: "Probe", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “Manual plural %lld” in table “Probe.xcstrings”.
         */
        static func manualPlural(_ arg1: Int) -> LocalizedStringResource {
            LocalizedStringResource("Manual plural %lld", defaultValue: "\(arg1, specifier: "%lld")", table: "Probe", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “Manual substitution %lld” in table “Probe.xcstrings”.
         */
        static func manualSubstitution(n: Int) -> LocalizedStringResource {
            LocalizedStringResource("Manual substitution %lld", defaultValue: "\(n, specifier: "%lld")", table: "Probe", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “Manual width” in table “Probe.xcstrings”.
         */
        static var manualWidth: LocalizedStringResource {
            LocalizedStringResource("Manual width", table: "Probe", bundle: resourceBundleDescription)
        }

        /**
         Localized string for key “Manual with no value” in table “Probe.xcstrings”.
         */
        static var manualWithNoValue: LocalizedStringResource {
            LocalizedStringResource("Manual with no value", table: "Probe", bundle: resourceBundleDescription)
        }
    }
}