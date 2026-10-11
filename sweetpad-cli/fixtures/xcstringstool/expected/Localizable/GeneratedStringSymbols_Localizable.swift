// 
// GeneratedStringSymbols_Localizable.swift
// Auto-Generated symbols for localized strings defined in “Localizable.xcstrings”.
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
    /**
     Unread messages
     
     Localized string for key “inbox.count %lld” in table “Localizable.xcstrings”.
     */
    static func inboxCount(_ arg1: Int) -> LocalizedStringResource {
        LocalizedStringResource("inbox.count %lld", defaultValue: "\(arg1, specifier: "%lld")", table: "Localizable", bundle: resourceBundleDescription)
    }

    /**
     Localized string for key “welcome.title” in table “Localizable.xcstrings”.
     */
    static var welcomeTitle: LocalizedStringResource {
        LocalizedStringResource("welcome.title", table: "Localizable", bundle: resourceBundleDescription)
    }
}