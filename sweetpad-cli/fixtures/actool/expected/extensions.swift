import Foundation
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(SwiftUI)
import SwiftUI
#endif
#if canImport(DeveloperToolsSupport)
import DeveloperToolsSupport
#endif

#if SWIFT_PACKAGE
private let resourceBundle = Foundation.Bundle.module
#else
private class ResourceBundleClass {}
private let resourceBundle = Foundation.Bundle(for: ResourceBundleClass.self)
#endif

// MARK: - Color Symbols -

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
extension DeveloperToolsSupport.ColorResource {

    /// The "#hash" asset catalog color resource.
    static let hash = DeveloperToolsSupport.ColorResource(name: "#hash", bundle: resourceBundle)

    /// The "1stColor" asset catalog color resource.
    static let _1St = DeveloperToolsSupport.ColorResource(name: "1stColor", bundle: resourceBundle)

    /// The "9" asset catalog color resource.
    static let _9 = DeveloperToolsSupport.ColorResource(name: "9", bundle: resourceBundle)

    /// The "ABCdef" asset catalog color resource.
    static let abCdef = DeveloperToolsSupport.ColorResource(name: "ABCdef", bundle: resourceBundle)

    /// The "ALLCAPS" asset catalog color resource.
    static let ALLCAPS = DeveloperToolsSupport.ColorResource(name: "ALLCAPS", bundle: resourceBundle)

    /// The "A_B" asset catalog color resource.
    static let A_B = DeveloperToolsSupport.ColorResource(name: "A_B", bundle: resourceBundle)

    /// The "AccentColor" asset catalog color resource.
    static let accent = DeveloperToolsSupport.ColorResource(name: "AccentColor", bundle: resourceBundle)

    /// The "BothIdioms" asset catalog color resource.
    static let bothIdioms = DeveloperToolsSupport.ColorResource(name: "BothIdioms", bundle: resourceBundle)

    /// The "Brand Yellow" asset catalog color resource.
    static let brandYellow = DeveloperToolsSupport.ColorResource(name: "Brand Yellow", bundle: resourceBundle)

    /// The "BrandRed" asset catalog color resource.
    static let brandRed = DeveloperToolsSupport.ColorResource(name: "BrandRed", bundle: resourceBundle)

    /// The "Color" asset catalog color resource.
    static let color = DeveloperToolsSupport.ColorResource(name: "Color", bundle: resourceBundle)

    /// The "Color.Dark" asset catalog color resource.
    static let colorDark = DeveloperToolsSupport.ColorResource(name: "Color.Dark", bundle: resourceBundle)

    #warning("The \"ColorColor\" color asset name resolves to the symbol \"color\" which already exists. Try renaming the asset.")

    /// The "Gray" asset catalog color resource.
    static let gray = DeveloperToolsSupport.ColorResource(name: "Gray", bundle: resourceBundle)

    /// The "Gray 100" asset catalog color resource.
    static let gray100 = DeveloperToolsSupport.ColorResource(name: "Gray 100", bundle: resourceBundle)

    /// The "HTML5Color" asset catalog color resource.
    static let HTML_5 = DeveloperToolsSupport.ColorResource(name: "HTML5Color", bundle: resourceBundle)

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Tint" asset catalog color resource.
        static let tint = DeveloperToolsSupport.ColorResource(name: "Icons/Tint", bundle: resourceBundle)

    }

    /// The "PhoneOnly" asset catalog color resource.
    @available(watchOS, unavailable)
    static let phoneOnly = DeveloperToolsSupport.ColorResource(thinnableName: "PhoneOnly", bundle: resourceBundle)

    /// The "Protocol" asset catalog color resource.
    static let `protocol` = DeveloperToolsSupport.ColorResource(name: "Protocol", bundle: resourceBundle)

    /// The "Type" asset catalog color resource.
    static let type = DeveloperToolsSupport.ColorResource(name: "Type", bundle: resourceBundle)

    /// The "UPPER_snake" asset catalog color resource.
    static let upperSnake = DeveloperToolsSupport.ColorResource(name: "UPPER_snake", bundle: resourceBundle)

    /// The "URLColor" asset catalog color resource.
    static let URL = DeveloperToolsSupport.ColorResource(name: "URLColor", bundle: resourceBundle)

    /// The "UniPhone" asset catalog color resource.
    static let uniPhone = DeveloperToolsSupport.ColorResource(name: "UniPhone", bundle: resourceBundle)

    /// The "_private" asset catalog color resource.
    static let `private` = DeveloperToolsSupport.ColorResource(name: "_private", bundle: resourceBundle)

    /// The "a" asset catalog color resource.
    static let a = DeveloperToolsSupport.ColorResource(name: "a", bundle: resourceBundle)

    #warning("The \"accent\" color asset name resolves to the symbol \"accent\" which already exists. Try renaming the asset.")

    /// The "alloc" asset catalog color resource.
    static let alloc = DeveloperToolsSupport.ColorResource(name: "alloc", bundle: resourceBundle)

    /// The "alternateSelectedControl" asset catalog color resource.
    static let alternateSelectedControl = DeveloperToolsSupport.ColorResource(name: "alternateSelectedControl", bundle: resourceBundle)

    /// The "alternateSelectedControlText" asset catalog color resource.
    static let alternateSelectedControlText = DeveloperToolsSupport.ColorResource(name: "alternateSelectedControlText", bundle: resourceBundle)

    /// The "amp&ersand" asset catalog color resource.
    static let ampErsand = DeveloperToolsSupport.ColorResource(name: "amp&ersand", bundle: resourceBundle)

    /// The "apostrophe\'s" asset catalog color resource.
    static let apostropheS = DeveloperToolsSupport.ColorResource(name: "apostrophe\'s", bundle: resourceBundle)

    /// The "black" asset catalog color resource.
    static let black = DeveloperToolsSupport.ColorResource(name: "black", bundle: resourceBundle)

    /// The "blue" asset catalog color resource.
    static let blue = DeveloperToolsSupport.ColorResource(name: "blue", bundle: resourceBundle)

    /// The "brand-blue" asset catalog color resource.
    static let brandBlue = DeveloperToolsSupport.ColorResource(name: "brand-blue", bundle: resourceBundle)

    /// The "brandRed2" asset catalog color resource.
    static let brandRed2 = DeveloperToolsSupport.ColorResource(name: "brandRed2", bundle: resourceBundle)

    /// The "brand_green" asset catalog color resource.
    static let brandGreen = DeveloperToolsSupport.ColorResource(name: "brand_green", bundle: resourceBundle)

    /// The "brown" asset catalog color resource.
    static let brown = DeveloperToolsSupport.ColorResource(name: "brown", bundle: resourceBundle)

    /// The "camel-Case" asset catalog color resource.
    static let camelCase = DeveloperToolsSupport.ColorResource(name: "camel-Case", bundle: resourceBundle)

    /// The "class" asset catalog color resource.
    static let `class` = DeveloperToolsSupport.ColorResource(name: "class", bundle: resourceBundle)

    /// The "clear" asset catalog color resource.
    static let clear = DeveloperToolsSupport.ColorResource(name: "clear", bundle: resourceBundle)

    /// The "comma,name" asset catalog color resource.
    static let commaName = DeveloperToolsSupport.ColorResource(name: "comma,name", bundle: resourceBundle)

    /// The "control" asset catalog color resource.
    static let control = DeveloperToolsSupport.ColorResource(name: "control", bundle: resourceBundle)

    /// The "controlAccent" asset catalog color resource.
    static let controlAccent = DeveloperToolsSupport.ColorResource(name: "controlAccent", bundle: resourceBundle)

    /// The "controlBackground" asset catalog color resource.
    static let controlBackground = DeveloperToolsSupport.ColorResource(name: "controlBackground", bundle: resourceBundle)

    /// The "controlDarkShadow" asset catalog color resource.
    static let controlDarkShadow = DeveloperToolsSupport.ColorResource(name: "controlDarkShadow", bundle: resourceBundle)

    /// The "controlHighlight" asset catalog color resource.
    static let controlHighlight = DeveloperToolsSupport.ColorResource(name: "controlHighlight", bundle: resourceBundle)

    /// The "controlLightHighlight" asset catalog color resource.
    static let controlLightHighlight = DeveloperToolsSupport.ColorResource(name: "controlLightHighlight", bundle: resourceBundle)

    /// The "controlShadow" asset catalog color resource.
    static let controlShadow = DeveloperToolsSupport.ColorResource(name: "controlShadow", bundle: resourceBundle)

    /// The "controlText" asset catalog color resource.
    static let controlText = DeveloperToolsSupport.ColorResource(name: "controlText", bundle: resourceBundle)

    /// The "copy" asset catalog color resource.
    static let copy = DeveloperToolsSupport.ColorResource(name: "copy", bundle: resourceBundle)

    /// The "cyan" asset catalog color resource.
    static let cyan = DeveloperToolsSupport.ColorResource(name: "cyan", bundle: resourceBundle)

    /// The "dark-mode" asset catalog resource namespace.
    enum DarkMode {

        /// The "dark-mode/Bg" asset catalog color resource.
        static let bg = DeveloperToolsSupport.ColorResource(name: "dark-mode/Bg", bundle: resourceBundle)

    }

    /// The "darkColor" asset catalog color resource.
    static let dark = DeveloperToolsSupport.ColorResource(name: "darkColor", bundle: resourceBundle)

    /// The "darkGray" asset catalog color resource.
    static let darkGray = DeveloperToolsSupport.ColorResource(name: "darkGray", bundle: resourceBundle)

    /// The "darkText" asset catalog color resource.
    static let darkText = DeveloperToolsSupport.ColorResource(name: "darkText", bundle: resourceBundle)

    /// The "default" asset catalog color resource.
    static let `default` = DeveloperToolsSupport.ColorResource(name: "default", bundle: resourceBundle)

    /// The "defaultValue" asset catalog color resource.
    static let defaultValue = DeveloperToolsSupport.ColorResource(name: "defaultValue", bundle: resourceBundle)

    /// The "description" asset catalog color resource.
    static let description = DeveloperToolsSupport.ColorResource(name: "description", bundle: resourceBundle)

    /// The "disabledControlText" asset catalog color resource.
    static let disabledControlText = DeveloperToolsSupport.ColorResource(name: "disabledControlText", bundle: resourceBundle)

    /// The "emoji🎉" asset catalog color resource.
    static let emoji = DeveloperToolsSupport.ColorResource(name: "emoji🎉", bundle: resourceBundle)

    /// The "findHighlight" asset catalog color resource.
    static let findHighlight = DeveloperToolsSupport.ColorResource(name: "findHighlight", bundle: resourceBundle)

    /// The "foo--bar" asset catalog color resource.
    static let fooBar = DeveloperToolsSupport.ColorResource(name: "foo--bar", bundle: resourceBundle)

    #warning("The \"foo__bar\" color asset name resolves to the symbol \"fooBar\" which already exists. Try renaming the asset.")

    #warning("The \"gray\" color asset name resolves to the symbol \"gray\" which already exists. Try renaming the asset.")

    /// The "green" asset catalog color resource.
    static let green = DeveloperToolsSupport.ColorResource(name: "green", bundle: resourceBundle)

    /// The "grid" asset catalog color resource.
    static let grid = DeveloperToolsSupport.ColorResource(name: "grid", bundle: resourceBundle)

    /// The "groupTableViewBackground" asset catalog color resource.
    static let groupTableViewBackground = DeveloperToolsSupport.ColorResource(name: "groupTableViewBackground", bundle: resourceBundle)

    #warning("The \"hash\" color asset name resolves to the symbol \"hash\" which already exists. Try renaming the asset.")

    /// The "header" asset catalog color resource.
    static let header = DeveloperToolsSupport.ColorResource(name: "header", bundle: resourceBundle)

    /// The "headerText" asset catalog color resource.
    static let headerText = DeveloperToolsSupport.ColorResource(name: "headerText", bundle: resourceBundle)

    /// The "highlight" asset catalog color resource.
    static let highlight = DeveloperToolsSupport.ColorResource(name: "highlight", bundle: resourceBundle)

    /// The "iOSColor" asset catalog color resource.
    static let iOS = DeveloperToolsSupport.ColorResource(name: "iOSColor", bundle: resourceBundle)

    /// The "indigo" asset catalog color resource.
    static let indigo = DeveloperToolsSupport.ColorResource(name: "indigo", bundle: resourceBundle)

    /// The "init" asset catalog color resource.
    static let `init` = DeveloperToolsSupport.ColorResource(name: "init", bundle: resourceBundle)

    /// The "keyboardFocusIndicator" asset catalog color resource.
    static let keyboardFocusIndicator = DeveloperToolsSupport.ColorResource(name: "keyboardFocusIndicator", bundle: resourceBundle)

    /// The "knob" asset catalog color resource.
    static let knob = DeveloperToolsSupport.ColorResource(name: "knob", bundle: resourceBundle)

    /// The "label" asset catalog color resource.
    static let label = DeveloperToolsSupport.ColorResource(name: "label", bundle: resourceBundle)

    /// The "label2" asset catalog color resource.
    static let label2 = DeveloperToolsSupport.ColorResource(name: "label2", bundle: resourceBundle)

    /// The "lightGray" asset catalog color resource.
    static let lightGray = DeveloperToolsSupport.ColorResource(name: "lightGray", bundle: resourceBundle)

    /// The "lightText" asset catalog color resource.
    static let lightText = DeveloperToolsSupport.ColorResource(name: "lightText", bundle: resourceBundle)

    /// The "link" asset catalog color resource.
    static let link = DeveloperToolsSupport.ColorResource(name: "link", bundle: resourceBundle)

    /// The "magenta" asset catalog color resource.
    static let magenta = DeveloperToolsSupport.ColorResource(name: "magenta", bundle: resourceBundle)

    /// The "mint" asset catalog color resource.
    static let mint = DeveloperToolsSupport.ColorResource(name: "mint", bundle: resourceBundle)

    /// The "myURLColor" asset catalog color resource.
    static let myURL = DeveloperToolsSupport.ColorResource(name: "myURLColor", bundle: resourceBundle)

    /// The "new" asset catalog color resource.
    static let new = DeveloperToolsSupport.ColorResource(name: "new", bundle: resourceBundle)

    /// The "nil" asset catalog color resource.
    static let `nil` = DeveloperToolsSupport.ColorResource(name: "nil", bundle: resourceBundle)

    /// The "opaqueSeparator" asset catalog color resource.
    static let opaqueSeparator = DeveloperToolsSupport.ColorResource(name: "opaqueSeparator", bundle: resourceBundle)

    /// The "orange" asset catalog color resource.
    static let orange = DeveloperToolsSupport.ColorResource(name: "orange", bundle: resourceBundle)

    /// The "paren(s)" asset catalog color resource.
    static let parenS = DeveloperToolsSupport.ColorResource(name: "paren(s)", bundle: resourceBundle)

    /// The "pink" asset catalog color resource.
    static let pink = DeveloperToolsSupport.ColorResource(name: "pink", bundle: resourceBundle)

    /// The "placeholderText" asset catalog color resource.
    static let placeholderText = DeveloperToolsSupport.ColorResource(name: "placeholderText", bundle: resourceBundle)

    /// The "plus+sign" asset catalog color resource.
    static let plusSign = DeveloperToolsSupport.ColorResource(name: "plus+sign", bundle: resourceBundle)

    /// The "primary" asset catalog color resource.
    static let primary = DeveloperToolsSupport.ColorResource(name: "primary", bundle: resourceBundle)

    /// The "purple" asset catalog color resource.
    static let purple = DeveloperToolsSupport.ColorResource(name: "purple", bundle: resourceBundle)

    /// The "quaternaryLabel" asset catalog color resource.
    static let quaternaryLabel = DeveloperToolsSupport.ColorResource(name: "quaternaryLabel", bundle: resourceBundle)

    /// The "quaternarySystemFill" asset catalog color resource.
    static let quaternarySystemFill = DeveloperToolsSupport.ColorResource(name: "quaternarySystemFill", bundle: resourceBundle)

    /// The "quinaryLabel" asset catalog color resource.
    static let quinaryLabel = DeveloperToolsSupport.ColorResource(name: "quinaryLabel", bundle: resourceBundle)

    /// The "quinarySystemFill" asset catalog color resource.
    static let quinarySystemFill = DeveloperToolsSupport.ColorResource(name: "quinarySystemFill", bundle: resourceBundle)

    /// The "red" asset catalog color resource.
    static let red = DeveloperToolsSupport.ColorResource(name: "red", bundle: resourceBundle)

    /// The "red-500" asset catalog color resource.
    static let red500 = DeveloperToolsSupport.ColorResource(name: "red-500", bundle: resourceBundle)

    /// The "scrollBar" asset catalog color resource.
    static let scrollBar = DeveloperToolsSupport.ColorResource(name: "scrollBar", bundle: resourceBundle)

    /// The "scrollViewTexturedBackground" asset catalog color resource.
    static let scrollViewTexturedBackground = DeveloperToolsSupport.ColorResource(name: "scrollViewTexturedBackground", bundle: resourceBundle)

    /// The "scrubberTexturedBackground" asset catalog color resource.
    static let scrubberTexturedBackground = DeveloperToolsSupport.ColorResource(name: "scrubberTexturedBackground", bundle: resourceBundle)

    /// The "secondary" asset catalog color resource.
    static let secondary = DeveloperToolsSupport.ColorResource(name: "secondary", bundle: resourceBundle)

    /// The "secondaryLabel" asset catalog color resource.
    static let secondaryLabel = DeveloperToolsSupport.ColorResource(name: "secondaryLabel", bundle: resourceBundle)

    /// The "secondarySelectedControl" asset catalog color resource.
    static let secondarySelectedControl = DeveloperToolsSupport.ColorResource(name: "secondarySelectedControl", bundle: resourceBundle)

    /// The "secondarySystemBackground" asset catalog color resource.
    static let secondarySystemBackground = DeveloperToolsSupport.ColorResource(name: "secondarySystemBackground", bundle: resourceBundle)

    /// The "secondarySystemFill" asset catalog color resource.
    static let secondarySystemFill = DeveloperToolsSupport.ColorResource(name: "secondarySystemFill", bundle: resourceBundle)

    /// The "secondarySystemGroupedBackground" asset catalog color resource.
    static let secondarySystemGroupedBackground = DeveloperToolsSupport.ColorResource(name: "secondarySystemGroupedBackground", bundle: resourceBundle)

    /// The "selectedContentBackground" asset catalog color resource.
    static let selectedContentBackground = DeveloperToolsSupport.ColorResource(name: "selectedContentBackground", bundle: resourceBundle)

    /// The "selectedControl" asset catalog color resource.
    static let selectedControl = DeveloperToolsSupport.ColorResource(name: "selectedControl", bundle: resourceBundle)

    /// The "selectedControlText" asset catalog color resource.
    static let selectedControlText = DeveloperToolsSupport.ColorResource(name: "selectedControlText", bundle: resourceBundle)

    /// The "selectedKnob" asset catalog color resource.
    static let selectedKnob = DeveloperToolsSupport.ColorResource(name: "selectedKnob", bundle: resourceBundle)

    /// The "selectedMenuItem" asset catalog color resource.
    static let selectedMenuItem = DeveloperToolsSupport.ColorResource(name: "selectedMenuItem", bundle: resourceBundle)

    /// The "selectedMenuItemText" asset catalog color resource.
    static let selectedMenuItemText = DeveloperToolsSupport.ColorResource(name: "selectedMenuItemText", bundle: resourceBundle)

    /// The "selectedText" asset catalog color resource.
    static let selectedText = DeveloperToolsSupport.ColorResource(name: "selectedText", bundle: resourceBundle)

    /// The "selectedTextBackground" asset catalog color resource.
    static let selectedTextBackground = DeveloperToolsSupport.ColorResource(name: "selectedTextBackground", bundle: resourceBundle)

    /// The "self" asset catalog color resource.
    static let `self` = DeveloperToolsSupport.ColorResource(name: "self", bundle: resourceBundle)

    /// The "separator" asset catalog color resource.
    static let separator = DeveloperToolsSupport.ColorResource(name: "separator", bundle: resourceBundle)

    /// The "shadow" asset catalog color resource.
    static let shadow = DeveloperToolsSupport.ColorResource(name: "shadow", bundle: resourceBundle)

    /// The "superclass" asset catalog color resource.
    static let superclass = DeveloperToolsSupport.ColorResource(name: "superclass", bundle: resourceBundle)

    /// The "systemBackground" asset catalog color resource.
    static let systemBackground = DeveloperToolsSupport.ColorResource(name: "systemBackground", bundle: resourceBundle)

    /// The "systemBlue" asset catalog color resource.
    static let systemBlue = DeveloperToolsSupport.ColorResource(name: "systemBlue", bundle: resourceBundle)

    /// The "systemBrown" asset catalog color resource.
    static let systemBrown = DeveloperToolsSupport.ColorResource(name: "systemBrown", bundle: resourceBundle)

    /// The "systemCyan" asset catalog color resource.
    static let systemCyan = DeveloperToolsSupport.ColorResource(name: "systemCyan", bundle: resourceBundle)

    /// The "systemFill" asset catalog color resource.
    static let systemFill = DeveloperToolsSupport.ColorResource(name: "systemFill", bundle: resourceBundle)

    /// The "systemGray" asset catalog color resource.
    static let systemGray = DeveloperToolsSupport.ColorResource(name: "systemGray", bundle: resourceBundle)

    /// The "systemGray2" asset catalog color resource.
    static let systemGray2 = DeveloperToolsSupport.ColorResource(name: "systemGray2", bundle: resourceBundle)

    /// The "systemGray3" asset catalog color resource.
    static let systemGray3 = DeveloperToolsSupport.ColorResource(name: "systemGray3", bundle: resourceBundle)

    /// The "systemGray4" asset catalog color resource.
    static let systemGray4 = DeveloperToolsSupport.ColorResource(name: "systemGray4", bundle: resourceBundle)

    /// The "systemGray5" asset catalog color resource.
    static let systemGray5 = DeveloperToolsSupport.ColorResource(name: "systemGray5", bundle: resourceBundle)

    /// The "systemGray6" asset catalog color resource.
    static let systemGray6 = DeveloperToolsSupport.ColorResource(name: "systemGray6", bundle: resourceBundle)

    /// The "systemGreen" asset catalog color resource.
    static let systemGreen = DeveloperToolsSupport.ColorResource(name: "systemGreen", bundle: resourceBundle)

    /// The "systemGroupedBackground" asset catalog color resource.
    static let systemGroupedBackground = DeveloperToolsSupport.ColorResource(name: "systemGroupedBackground", bundle: resourceBundle)

    /// The "systemIndigo" asset catalog color resource.
    static let systemIndigo = DeveloperToolsSupport.ColorResource(name: "systemIndigo", bundle: resourceBundle)

    /// The "systemMint" asset catalog color resource.
    static let systemMint = DeveloperToolsSupport.ColorResource(name: "systemMint", bundle: resourceBundle)

    /// The "systemOrange" asset catalog color resource.
    static let systemOrange = DeveloperToolsSupport.ColorResource(name: "systemOrange", bundle: resourceBundle)

    /// The "systemPink" asset catalog color resource.
    static let systemPink = DeveloperToolsSupport.ColorResource(name: "systemPink", bundle: resourceBundle)

    /// The "systemPurple" asset catalog color resource.
    static let systemPurple = DeveloperToolsSupport.ColorResource(name: "systemPurple", bundle: resourceBundle)

    /// The "systemRed" asset catalog color resource.
    static let systemRed = DeveloperToolsSupport.ColorResource(name: "systemRed", bundle: resourceBundle)

    /// The "systemTeal" asset catalog color resource.
    static let systemTeal = DeveloperToolsSupport.ColorResource(name: "systemTeal", bundle: resourceBundle)

    /// The "systemYellow" asset catalog color resource.
    static let systemYellow = DeveloperToolsSupport.ColorResource(name: "systemYellow", bundle: resourceBundle)

    /// The "teal" asset catalog color resource.
    static let teal = DeveloperToolsSupport.ColorResource(name: "teal", bundle: resourceBundle)

    /// The "tertiaryLabel" asset catalog color resource.
    static let tertiaryLabel = DeveloperToolsSupport.ColorResource(name: "tertiaryLabel", bundle: resourceBundle)

    /// The "tertiarySystemBackground" asset catalog color resource.
    static let tertiarySystemBackground = DeveloperToolsSupport.ColorResource(name: "tertiarySystemBackground", bundle: resourceBundle)

    /// The "tertiarySystemFill" asset catalog color resource.
    static let tertiarySystemFill = DeveloperToolsSupport.ColorResource(name: "tertiarySystemFill", bundle: resourceBundle)

    /// The "tertiarySystemGroupedBackground" asset catalog color resource.
    static let tertiarySystemGroupedBackground = DeveloperToolsSupport.ColorResource(name: "tertiarySystemGroupedBackground", bundle: resourceBundle)

    /// The "text" asset catalog color resource.
    static let text = DeveloperToolsSupport.ColorResource(name: "text", bundle: resourceBundle)

    /// The "textBackground" asset catalog color resource.
    static let textBackground = DeveloperToolsSupport.ColorResource(name: "textBackground", bundle: resourceBundle)

    /// The "textInsertionPoint" asset catalog color resource.
    static let textInsertionPoint = DeveloperToolsSupport.ColorResource(name: "textInsertionPoint", bundle: resourceBundle)

    /// The "tint" asset catalog color resource.
    static let tint = DeveloperToolsSupport.ColorResource(name: "tint", bundle: resourceBundle)

    /// The "true" asset catalog color resource.
    static let `true` = DeveloperToolsSupport.ColorResource(name: "true", bundle: resourceBundle)

    /// The "underPageBackground" asset catalog color resource.
    static let underPageBackground = DeveloperToolsSupport.ColorResource(name: "underPageBackground", bundle: resourceBundle)

    /// The "unemphasizedSelectedContentBackground" asset catalog color resource.
    static let unemphasizedSelectedContentBackground = DeveloperToolsSupport.ColorResource(name: "unemphasizedSelectedContentBackground", bundle: resourceBundle)

    /// The "unemphasizedSelectedText" asset catalog color resource.
    static let unemphasizedSelectedText = DeveloperToolsSupport.ColorResource(name: "unemphasizedSelectedText", bundle: resourceBundle)

    /// The "unemphasizedSelectedTextBackground" asset catalog color resource.
    static let unemphasizedSelectedTextBackground = DeveloperToolsSupport.ColorResource(name: "unemphasizedSelectedTextBackground", bundle: resourceBundle)

    /// The "viewFlipsideBackground" asset catalog color resource.
    static let viewFlipsideBackground = DeveloperToolsSupport.ColorResource(name: "viewFlipsideBackground", bundle: resourceBundle)

    /// The "white" asset catalog color resource.
    static let white = DeveloperToolsSupport.ColorResource(name: "white", bundle: resourceBundle)

    /// The "windowBackground" asset catalog color resource.
    static let windowBackground = DeveloperToolsSupport.ColorResource(name: "windowBackground", bundle: resourceBundle)

    /// The "windowFrame" asset catalog color resource.
    static let windowFrame = DeveloperToolsSupport.ColorResource(name: "windowFrame", bundle: resourceBundle)

    /// The "windowFrameText" asset catalog color resource.
    static let windowFrameText = DeveloperToolsSupport.ColorResource(name: "windowFrameText", bundle: resourceBundle)

    /// The "with.dots.many" asset catalog color resource.
    static let withDotsMany = DeveloperToolsSupport.ColorResource(name: "with.dots.many", bundle: resourceBundle)

    /// The "x1y2" asset catalog color resource.
    static let x1Y2 = DeveloperToolsSupport.ColorResource(name: "x1y2", bundle: resourceBundle)

    /// The "yellow" asset catalog color resource.
    static let yellow = DeveloperToolsSupport.ColorResource(name: "yellow", bundle: resourceBundle)

    /// The "zero" asset catalog color resource.
    static let zero = DeveloperToolsSupport.ColorResource(name: "zero", bundle: resourceBundle)

    /// The "Ünïcode" asset catalog color resource.
    static let ünïcode = DeveloperToolsSupport.ColorResource(name: "Ünïcode", bundle: resourceBundle)

}

// MARK: - Image Symbols -

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
extension DeveloperToolsSupport.ImageResource {

    /// The "1stImage" asset catalog image resource.
    static let _1St = DeveloperToolsSupport.ImageResource(name: "1stImage", bundle: resourceBundle)

    /// The "BigImageView" asset catalog image resource.
    static let bigImageView = DeveloperToolsSupport.ImageResource(name: "BigImageView", bundle: resourceBundle)

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Star" asset catalog image resource.
        static let star = DeveloperToolsSupport.ImageResource(name: "Icons/Star", bundle: resourceBundle)

    }

    /// The "Image" asset catalog image resource.
    static let image = DeveloperToolsSupport.ImageResource(name: "Image", bundle: resourceBundle)

    /// The "Image Name" asset catalog image resource.
    static let imageName = DeveloperToolsSupport.ImageResource(name: "Image Name", bundle: resourceBundle)

    /// The "Image2" asset catalog image resource.
    static let image2 = DeveloperToolsSupport.ImageResource(name: "Image2", bundle: resourceBundle)

    /// The "Logo" asset catalog image resource.
    static let logo = DeveloperToolsSupport.ImageResource(name: "Logo", bundle: resourceBundle)

    #warning("The \"LogoImage\" image asset name resolves to the symbol \"logo\" which already exists. Try renaming the asset.")

    /// The "Nested" asset catalog resource namespace.
    enum Nested {

        /// The "Nested/Deep" asset catalog resource namespace.
        enum Deep {

            /// The "Nested/Deep/Leaf" asset catalog image resource.
            static let leaf = DeveloperToolsSupport.ImageResource(name: "Nested/Deep/Leaf", bundle: resourceBundle)

        }

    }

    /// The "PadOnly" asset catalog image resource.
    @available(watchOS, unavailable)
    static let padOnly = DeveloperToolsSupport.ImageResource(thinnableName: "PadOnly", bundle: resourceBundle)

    /// The "actions" asset catalog image resource.
    static let actions = DeveloperToolsSupport.ImageResource(name: "actions", bundle: resourceBundle)

    /// The "add" asset catalog image resource.
    static let add = DeveloperToolsSupport.ImageResource(name: "add", bundle: resourceBundle)

    /// The "branch" asset catalog image resource.
    static let branch = DeveloperToolsSupport.ImageResource(name: "branch", bundle: resourceBundle)

    /// The "checkmark" asset catalog image resource.
    static let checkmark = DeveloperToolsSupport.ImageResource(name: "checkmark", bundle: resourceBundle)

    /// The "highlightedBranch" asset catalog image resource.
    static let highlightedBranch = DeveloperToolsSupport.ImageResource(name: "highlightedBranch", bundle: resourceBundle)

    /// The "init" asset catalog image resource.
    static let `init` = DeveloperToolsSupport.ImageResource(name: "init", bundle: resourceBundle)

    /// The "logo-dark" asset catalog image resource.
    static let logoDark = DeveloperToolsSupport.ImageResource(name: "logo-dark", bundle: resourceBundle)

    /// The "my icons" asset catalog resource namespace.
    enum MyIcons {

        /// The "my icons/Home" asset catalog image resource.
        static let home = DeveloperToolsSupport.ImageResource(name: "my icons/Home", bundle: resourceBundle)

    }

    /// The "photo" asset catalog image resource.
    static let photo = DeveloperToolsSupport.ImageResource(name: "photo", bundle: resourceBundle)

    /// The "remove" asset catalog image resource.
    static let remove = DeveloperToolsSupport.ImageResource(name: "remove", bundle: resourceBundle)

    /// The "self" asset catalog image resource.
    static let `self` = DeveloperToolsSupport.ImageResource(name: "self", bundle: resourceBundle)

    /// The "strokedCheckmark" asset catalog image resource.
    static let strokedCheckmark = DeveloperToolsSupport.ImageResource(name: "strokedCheckmark", bundle: resourceBundle)

    /// The "system" asset catalog image resource.
    static let system = DeveloperToolsSupport.ImageResource(name: "system", bundle: resourceBundle)

}

// MARK: - Color Symbol Extensions -

#if canImport(AppKit)
@available(macOS 14.0, *)
@available(macCatalyst, unavailable)
extension AppKit.NSColor {

    /// The "#hash" asset catalog color.
    static var hash: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .hash)
#else
        .init()
#endif
    }

    /// The "1stColor" asset catalog color.
    static var _1St: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: ._1St)
#else
        .init()
#endif
    }

    /// The "9" asset catalog color.
    static var _9: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: ._9)
#else
        .init()
#endif
    }

    /// The "ABCdef" asset catalog color.
    static var abCdef: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .abCdef)
#else
        .init()
#endif
    }

    /// The "ALLCAPS" asset catalog color.
    static var ALLCAPS: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .ALLCAPS)
#else
        .init()
#endif
    }

    /// The "A_B" asset catalog color.
    static var A_B: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .A_B)
#else
        .init()
#endif
    }

    /// The "AccentColor" asset catalog color.
    static var accent: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .accent)
#else
        .init()
#endif
    }

    /// The "BothIdioms" asset catalog color.
    static var bothIdioms: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .bothIdioms)
#else
        .init()
#endif
    }

    /// The "Brand Yellow" asset catalog color.
    static var brandYellow: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .brandYellow)
#else
        .init()
#endif
    }

    /// The "BrandRed" asset catalog color.
    static var brandRed: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .brandRed)
#else
        .init()
#endif
    }

    /// The "Color" asset catalog color.
    static var color: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .color)
#else
        .init()
#endif
    }

    /// The "Color.Dark" asset catalog color.
    static var colorDark: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .colorDark)
#else
        .init()
#endif
    }

    #warning("The \"Gray\" color asset name resolves to a conflicting NSColor symbol \"gray\". Try renaming the asset.")

    /// The "Gray 100" asset catalog color.
    static var gray100: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .gray100)
#else
        .init()
#endif
    }

    /// The "HTML5Color" asset catalog color.
    static var HTML_5: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .HTML_5)
#else
        .init()
#endif
    }

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Tint" asset catalog color.
        static var tint: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
            .init(resource: .Icons.tint)
#else
            .init()
#endif
        }

    }

    /// The "PhoneOnly" asset catalog color.
    static var phoneOnly: AppKit.NSColor? {
#if !targetEnvironment(macCatalyst)
        .init(thinnableResource: .phoneOnly)
#else
        .init()
#endif
    }

    /// The "Protocol" asset catalog color.
    static var `protocol`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`protocol`)
#else
        .init()
#endif
    }

    /// The "Type" asset catalog color.
    static var type: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .type)
#else
        .init()
#endif
    }

    /// The "UPPER_snake" asset catalog color.
    static var upperSnake: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .upperSnake)
#else
        .init()
#endif
    }

    /// The "URLColor" asset catalog color.
    static var URL: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .URL)
#else
        .init()
#endif
    }

    /// The "UniPhone" asset catalog color.
    static var uniPhone: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .uniPhone)
#else
        .init()
#endif
    }

    /// The "_private" asset catalog color.
    static var `private`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`private`)
#else
        .init()
#endif
    }

    /// The "a" asset catalog color.
    static var a: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .a)
#else
        .init()
#endif
    }

    /// The "alloc" asset catalog color.
    static var alloc: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .alloc)
#else
        .init()
#endif
    }

    /// The "alternateSelectedControl" asset catalog color.
    static var alternateSelectedControl: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .alternateSelectedControl)
#else
        .init()
#endif
    }

    /// The "alternateSelectedControlText" asset catalog color.
    static var alternateSelectedControlText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .alternateSelectedControlText)
#else
        .init()
#endif
    }

    /// The "amp&ersand" asset catalog color.
    static var ampErsand: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .ampErsand)
#else
        .init()
#endif
    }

    /// The "apostrophe\'s" asset catalog color.
    static var apostropheS: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .apostropheS)
#else
        .init()
#endif
    }

    #warning("The \"black\" color asset name resolves to a conflicting NSColor symbol \"black\". Try renaming the asset.")

    #warning("The \"blue\" color asset name resolves to a conflicting NSColor symbol \"blue\". Try renaming the asset.")

    /// The "brand-blue" asset catalog color.
    static var brandBlue: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .brandBlue)
#else
        .init()
#endif
    }

    /// The "brandRed2" asset catalog color.
    static var brandRed2: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .brandRed2)
#else
        .init()
#endif
    }

    /// The "brand_green" asset catalog color.
    static var brandGreen: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .brandGreen)
#else
        .init()
#endif
    }

    #warning("The \"brown\" color asset name resolves to a conflicting NSColor symbol \"brown\". Try renaming the asset.")

    /// The "camel-Case" asset catalog color.
    static var camelCase: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .camelCase)
#else
        .init()
#endif
    }

    /// The "class" asset catalog color.
    static var `class`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`class`)
#else
        .init()
#endif
    }

    #warning("The \"clear\" color asset name resolves to a conflicting NSColor symbol \"clear\". Try renaming the asset.")

    /// The "comma,name" asset catalog color.
    static var commaName: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .commaName)
#else
        .init()
#endif
    }

    /// The "control" asset catalog color.
    static var control: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .control)
#else
        .init()
#endif
    }

    /// The "controlAccent" asset catalog color.
    static var controlAccent: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlAccent)
#else
        .init()
#endif
    }

    /// The "controlBackground" asset catalog color.
    static var controlBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlBackground)
#else
        .init()
#endif
    }

    /// The "controlDarkShadow" asset catalog color.
    static var controlDarkShadow: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlDarkShadow)
#else
        .init()
#endif
    }

    /// The "controlHighlight" asset catalog color.
    static var controlHighlight: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlHighlight)
#else
        .init()
#endif
    }

    /// The "controlLightHighlight" asset catalog color.
    static var controlLightHighlight: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlLightHighlight)
#else
        .init()
#endif
    }

    /// The "controlShadow" asset catalog color.
    static var controlShadow: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlShadow)
#else
        .init()
#endif
    }

    /// The "controlText" asset catalog color.
    static var controlText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .controlText)
#else
        .init()
#endif
    }

    /// The "copy" asset catalog color.
    static var copy: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .copy)
#else
        .init()
#endif
    }

    #warning("The \"cyan\" color asset name resolves to a conflicting NSColor symbol \"cyan\". Try renaming the asset.")

    /// The "dark-mode" asset catalog resource namespace.
    enum DarkMode {

        /// The "dark-mode/Bg" asset catalog color.
        static var bg: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
            .init(resource: .DarkMode.bg)
#else
            .init()
#endif
        }

    }

    /// The "darkColor" asset catalog color.
    static var dark: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .dark)
#else
        .init()
#endif
    }

    #warning("The \"darkGray\" color asset name resolves to a conflicting NSColor symbol \"darkGray\". Try renaming the asset.")

    /// The "darkText" asset catalog color.
    static var darkText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .darkText)
#else
        .init()
#endif
    }

    /// The "default" asset catalog color.
    static var `default`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`default`)
#else
        .init()
#endif
    }

    /// The "defaultValue" asset catalog color.
    static var defaultValue: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .defaultValue)
#else
        .init()
#endif
    }

    /// The "description" asset catalog color.
    static var description: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .description)
#else
        .init()
#endif
    }

    /// The "disabledControlText" asset catalog color.
    static var disabledControlText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .disabledControlText)
#else
        .init()
#endif
    }

    /// The "emoji🎉" asset catalog color.
    static var emoji: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .emoji)
#else
        .init()
#endif
    }

    /// The "findHighlight" asset catalog color.
    static var findHighlight: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .findHighlight)
#else
        .init()
#endif
    }

    /// The "foo--bar" asset catalog color.
    static var fooBar: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .fooBar)
#else
        .init()
#endif
    }

    #warning("The \"green\" color asset name resolves to a conflicting NSColor symbol \"green\". Try renaming the asset.")

    /// The "grid" asset catalog color.
    static var grid: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .grid)
#else
        .init()
#endif
    }

    /// The "groupTableViewBackground" asset catalog color.
    static var groupTableViewBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .groupTableViewBackground)
#else
        .init()
#endif
    }

    /// The "header" asset catalog color.
    static var header: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .header)
#else
        .init()
#endif
    }

    /// The "headerText" asset catalog color.
    static var headerText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .headerText)
#else
        .init()
#endif
    }

    /// The "highlight" asset catalog color.
    static var highlight: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .highlight)
#else
        .init()
#endif
    }

    /// The "iOSColor" asset catalog color.
    static var iOS: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .iOS)
#else
        .init()
#endif
    }

    /// The "indigo" asset catalog color.
    static var indigo: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .indigo)
#else
        .init()
#endif
    }

    /// The "init" asset catalog color.
    static var `init`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`init`)
#else
        .init()
#endif
    }

    /// The "keyboardFocusIndicator" asset catalog color.
    static var keyboardFocusIndicator: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .keyboardFocusIndicator)
#else
        .init()
#endif
    }

    /// The "knob" asset catalog color.
    static var knob: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .knob)
#else
        .init()
#endif
    }

    /// The "label" asset catalog color.
    static var label: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .label)
#else
        .init()
#endif
    }

    /// The "label2" asset catalog color.
    static var label2: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .label2)
#else
        .init()
#endif
    }

    #warning("The \"lightGray\" color asset name resolves to a conflicting NSColor symbol \"lightGray\". Try renaming the asset.")

    /// The "lightText" asset catalog color.
    static var lightText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .lightText)
#else
        .init()
#endif
    }

    /// The "link" asset catalog color.
    static var link: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .link)
#else
        .init()
#endif
    }

    #warning("The \"magenta\" color asset name resolves to a conflicting NSColor symbol \"magenta\". Try renaming the asset.")

    /// The "mint" asset catalog color.
    static var mint: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .mint)
#else
        .init()
#endif
    }

    /// The "myURLColor" asset catalog color.
    static var myURL: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .myURL)
#else
        .init()
#endif
    }

    /// The "new" asset catalog color.
    static var new: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .new)
#else
        .init()
#endif
    }

    /// The "nil" asset catalog color.
    static var `nil`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`nil`)
#else
        .init()
#endif
    }

    /// The "opaqueSeparator" asset catalog color.
    static var opaqueSeparator: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .opaqueSeparator)
#else
        .init()
#endif
    }

    #warning("The \"orange\" color asset name resolves to a conflicting NSColor symbol \"orange\". Try renaming the asset.")

    /// The "paren(s)" asset catalog color.
    static var parenS: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .parenS)
#else
        .init()
#endif
    }

    /// The "pink" asset catalog color.
    static var pink: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .pink)
#else
        .init()
#endif
    }

    /// The "placeholderText" asset catalog color.
    static var placeholderText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .placeholderText)
#else
        .init()
#endif
    }

    /// The "plus+sign" asset catalog color.
    static var plusSign: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .plusSign)
#else
        .init()
#endif
    }

    /// The "primary" asset catalog color.
    static var primary: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .primary)
#else
        .init()
#endif
    }

    #warning("The \"purple\" color asset name resolves to a conflicting NSColor symbol \"purple\". Try renaming the asset.")

    /// The "quaternaryLabel" asset catalog color.
    static var quaternaryLabel: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .quaternaryLabel)
#else
        .init()
#endif
    }

    /// The "quaternarySystemFill" asset catalog color.
    static var quaternarySystemFill: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .quaternarySystemFill)
#else
        .init()
#endif
    }

    /// The "quinaryLabel" asset catalog color.
    static var quinaryLabel: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .quinaryLabel)
#else
        .init()
#endif
    }

    /// The "quinarySystemFill" asset catalog color.
    static var quinarySystemFill: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .quinarySystemFill)
#else
        .init()
#endif
    }

    #warning("The \"red\" color asset name resolves to a conflicting NSColor symbol \"red\". Try renaming the asset.")

    /// The "red-500" asset catalog color.
    static var red500: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .red500)
#else
        .init()
#endif
    }

    /// The "scrollBar" asset catalog color.
    static var scrollBar: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .scrollBar)
#else
        .init()
#endif
    }

    /// The "scrollViewTexturedBackground" asset catalog color.
    static var scrollViewTexturedBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .scrollViewTexturedBackground)
#else
        .init()
#endif
    }

    #warning("The \"scrubberTexturedBackground\" color asset name resolves to a conflicting NSColor symbol \"scrubberTexturedBackground\". Try renaming the asset.")

    /// The "secondary" asset catalog color.
    static var secondary: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .secondary)
#else
        .init()
#endif
    }

    /// The "secondaryLabel" asset catalog color.
    static var secondaryLabel: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .secondaryLabel)
#else
        .init()
#endif
    }

    /// The "secondarySelectedControl" asset catalog color.
    static var secondarySelectedControl: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .secondarySelectedControl)
#else
        .init()
#endif
    }

    /// The "secondarySystemBackground" asset catalog color.
    static var secondarySystemBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .secondarySystemBackground)
#else
        .init()
#endif
    }

    /// The "secondarySystemFill" asset catalog color.
    static var secondarySystemFill: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .secondarySystemFill)
#else
        .init()
#endif
    }

    /// The "secondarySystemGroupedBackground" asset catalog color.
    static var secondarySystemGroupedBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .secondarySystemGroupedBackground)
#else
        .init()
#endif
    }

    /// The "selectedContentBackground" asset catalog color.
    static var selectedContentBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedContentBackground)
#else
        .init()
#endif
    }

    /// The "selectedControl" asset catalog color.
    static var selectedControl: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedControl)
#else
        .init()
#endif
    }

    /// The "selectedControlText" asset catalog color.
    static var selectedControlText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedControlText)
#else
        .init()
#endif
    }

    /// The "selectedKnob" asset catalog color.
    static var selectedKnob: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedKnob)
#else
        .init()
#endif
    }

    /// The "selectedMenuItem" asset catalog color.
    static var selectedMenuItem: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedMenuItem)
#else
        .init()
#endif
    }

    /// The "selectedMenuItemText" asset catalog color.
    static var selectedMenuItemText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedMenuItemText)
#else
        .init()
#endif
    }

    /// The "selectedText" asset catalog color.
    static var selectedText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedText)
#else
        .init()
#endif
    }

    /// The "selectedTextBackground" asset catalog color.
    static var selectedTextBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .selectedTextBackground)
#else
        .init()
#endif
    }

    /// The "self" asset catalog color.
    static var `self`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`self`)
#else
        .init()
#endif
    }

    /// The "separator" asset catalog color.
    static var separator: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .separator)
#else
        .init()
#endif
    }

    /// The "shadow" asset catalog color.
    static var shadow: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .shadow)
#else
        .init()
#endif
    }

    /// The "superclass" asset catalog color.
    static var superclass: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .superclass)
#else
        .init()
#endif
    }

    /// The "systemBackground" asset catalog color.
    static var systemBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemBackground)
#else
        .init()
#endif
    }

    #warning("The \"systemBlue\" color asset name resolves to a conflicting NSColor symbol \"systemBlue\". Try renaming the asset.")

    #warning("The \"systemBrown\" color asset name resolves to a conflicting NSColor symbol \"systemBrown\". Try renaming the asset.")

    #warning("The \"systemCyan\" color asset name resolves to a conflicting NSColor symbol \"systemCyan\". Try renaming the asset.")

    /// The "systemFill" asset catalog color.
    static var systemFill: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemFill)
#else
        .init()
#endif
    }

    #warning("The \"systemGray\" color asset name resolves to a conflicting NSColor symbol \"systemGray\". Try renaming the asset.")

    /// The "systemGray2" asset catalog color.
    static var systemGray2: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemGray2)
#else
        .init()
#endif
    }

    /// The "systemGray3" asset catalog color.
    static var systemGray3: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemGray3)
#else
        .init()
#endif
    }

    /// The "systemGray4" asset catalog color.
    static var systemGray4: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemGray4)
#else
        .init()
#endif
    }

    /// The "systemGray5" asset catalog color.
    static var systemGray5: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemGray5)
#else
        .init()
#endif
    }

    /// The "systemGray6" asset catalog color.
    static var systemGray6: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemGray6)
#else
        .init()
#endif
    }

    #warning("The \"systemGreen\" color asset name resolves to a conflicting NSColor symbol \"systemGreen\". Try renaming the asset.")

    /// The "systemGroupedBackground" asset catalog color.
    static var systemGroupedBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .systemGroupedBackground)
#else
        .init()
#endif
    }

    #warning("The \"systemIndigo\" color asset name resolves to a conflicting NSColor symbol \"systemIndigo\". Try renaming the asset.")

    #warning("The \"systemMint\" color asset name resolves to a conflicting NSColor symbol \"systemMint\". Try renaming the asset.")

    #warning("The \"systemOrange\" color asset name resolves to a conflicting NSColor symbol \"systemOrange\". Try renaming the asset.")

    #warning("The \"systemPink\" color asset name resolves to a conflicting NSColor symbol \"systemPink\". Try renaming the asset.")

    #warning("The \"systemPurple\" color asset name resolves to a conflicting NSColor symbol \"systemPurple\". Try renaming the asset.")

    #warning("The \"systemRed\" color asset name resolves to a conflicting NSColor symbol \"systemRed\". Try renaming the asset.")

    #warning("The \"systemTeal\" color asset name resolves to a conflicting NSColor symbol \"systemTeal\". Try renaming the asset.")

    #warning("The \"systemYellow\" color asset name resolves to a conflicting NSColor symbol \"systemYellow\". Try renaming the asset.")

    /// The "teal" asset catalog color.
    static var teal: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .teal)
#else
        .init()
#endif
    }

    /// The "tertiaryLabel" asset catalog color.
    static var tertiaryLabel: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .tertiaryLabel)
#else
        .init()
#endif
    }

    /// The "tertiarySystemBackground" asset catalog color.
    static var tertiarySystemBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .tertiarySystemBackground)
#else
        .init()
#endif
    }

    /// The "tertiarySystemFill" asset catalog color.
    static var tertiarySystemFill: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .tertiarySystemFill)
#else
        .init()
#endif
    }

    /// The "tertiarySystemGroupedBackground" asset catalog color.
    static var tertiarySystemGroupedBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .tertiarySystemGroupedBackground)
#else
        .init()
#endif
    }

    /// The "text" asset catalog color.
    static var text: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .text)
#else
        .init()
#endif
    }

    /// The "textBackground" asset catalog color.
    static var textBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .textBackground)
#else
        .init()
#endif
    }

    /// The "textInsertionPoint" asset catalog color.
    static var textInsertionPoint: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .textInsertionPoint)
#else
        .init()
#endif
    }

    /// The "tint" asset catalog color.
    static var tint: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .tint)
#else
        .init()
#endif
    }

    /// The "true" asset catalog color.
    static var `true`: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`true`)
#else
        .init()
#endif
    }

    /// The "underPageBackground" asset catalog color.
    static var underPageBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .underPageBackground)
#else
        .init()
#endif
    }

    /// The "unemphasizedSelectedContentBackground" asset catalog color.
    static var unemphasizedSelectedContentBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .unemphasizedSelectedContentBackground)
#else
        .init()
#endif
    }

    /// The "unemphasizedSelectedText" asset catalog color.
    static var unemphasizedSelectedText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .unemphasizedSelectedText)
#else
        .init()
#endif
    }

    /// The "unemphasizedSelectedTextBackground" asset catalog color.
    static var unemphasizedSelectedTextBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .unemphasizedSelectedTextBackground)
#else
        .init()
#endif
    }

    /// The "viewFlipsideBackground" asset catalog color.
    static var viewFlipsideBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .viewFlipsideBackground)
#else
        .init()
#endif
    }

    #warning("The \"white\" color asset name resolves to a conflicting NSColor symbol \"white\". Try renaming the asset.")

    /// The "windowBackground" asset catalog color.
    static var windowBackground: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .windowBackground)
#else
        .init()
#endif
    }

    /// The "windowFrame" asset catalog color.
    static var windowFrame: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .windowFrame)
#else
        .init()
#endif
    }

    /// The "windowFrameText" asset catalog color.
    static var windowFrameText: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .windowFrameText)
#else
        .init()
#endif
    }

    /// The "with.dots.many" asset catalog color.
    static var withDotsMany: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .withDotsMany)
#else
        .init()
#endif
    }

    /// The "x1y2" asset catalog color.
    static var x1Y2: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .x1Y2)
#else
        .init()
#endif
    }

    #warning("The \"yellow\" color asset name resolves to a conflicting NSColor symbol \"yellow\". Try renaming the asset.")

    /// The "zero" asset catalog color.
    static var zero: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .zero)
#else
        .init()
#endif
    }

    /// The "Ünïcode" asset catalog color.
    static var ünïcode: AppKit.NSColor {
#if !targetEnvironment(macCatalyst)
        .init(resource: .ünïcode)
#else
        .init()
#endif
    }

}
#endif

#if canImport(UIKit)
@available(iOS 17.0, tvOS 17.0, *)
@available(watchOS, unavailable)
extension UIKit.UIColor {

    /// The "#hash" asset catalog color.
    static var hash: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .hash)
#else
        .init()
#endif
    }

    /// The "1stColor" asset catalog color.
    static var _1St: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: ._1St)
#else
        .init()
#endif
    }

    /// The "9" asset catalog color.
    static var _9: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: ._9)
#else
        .init()
#endif
    }

    /// The "ABCdef" asset catalog color.
    static var abCdef: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .abCdef)
#else
        .init()
#endif
    }

    /// The "ALLCAPS" asset catalog color.
    static var ALLCAPS: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .ALLCAPS)
#else
        .init()
#endif
    }

    /// The "A_B" asset catalog color.
    static var A_B: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .A_B)
#else
        .init()
#endif
    }

    /// The "AccentColor" asset catalog color.
    static var accent: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .accent)
#else
        .init()
#endif
    }

    /// The "BothIdioms" asset catalog color.
    static var bothIdioms: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .bothIdioms)
#else
        .init()
#endif
    }

    /// The "Brand Yellow" asset catalog color.
    static var brandYellow: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .brandYellow)
#else
        .init()
#endif
    }

    /// The "BrandRed" asset catalog color.
    static var brandRed: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .brandRed)
#else
        .init()
#endif
    }

    /// The "Color" asset catalog color.
    static var color: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .color)
#else
        .init()
#endif
    }

    /// The "Color.Dark" asset catalog color.
    static var colorDark: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .colorDark)
#else
        .init()
#endif
    }

    #warning("The \"Gray\" color asset name resolves to a conflicting UIColor symbol \"gray\". Try renaming the asset.")

    /// The "Gray 100" asset catalog color.
    static var gray100: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .gray100)
#else
        .init()
#endif
    }

    /// The "HTML5Color" asset catalog color.
    static var HTML_5: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .HTML_5)
#else
        .init()
#endif
    }

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Tint" asset catalog color.
        static var tint: UIKit.UIColor {
#if !os(watchOS)
            .init(resource: .Icons.tint)
#else
            .init()
#endif
        }

    }

    /// The "PhoneOnly" asset catalog color.
    static var phoneOnly: UIKit.UIColor? {
#if !os(watchOS)
        .init(thinnableResource: .phoneOnly)
#else
        .init()
#endif
    }

    /// The "Protocol" asset catalog color.
    static var `protocol`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`protocol`)
#else
        .init()
#endif
    }

    /// The "Type" asset catalog color.
    static var type: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .type)
#else
        .init()
#endif
    }

    /// The "UPPER_snake" asset catalog color.
    static var upperSnake: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .upperSnake)
#else
        .init()
#endif
    }

    /// The "URLColor" asset catalog color.
    static var URL: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .URL)
#else
        .init()
#endif
    }

    /// The "UniPhone" asset catalog color.
    static var uniPhone: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .uniPhone)
#else
        .init()
#endif
    }

    /// The "_private" asset catalog color.
    static var `private`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`private`)
#else
        .init()
#endif
    }

    /// The "a" asset catalog color.
    static var a: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .a)
#else
        .init()
#endif
    }

    /// The "alloc" asset catalog color.
    static var alloc: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .alloc)
#else
        .init()
#endif
    }

    /// The "alternateSelectedControl" asset catalog color.
    static var alternateSelectedControl: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .alternateSelectedControl)
#else
        .init()
#endif
    }

    /// The "alternateSelectedControlText" asset catalog color.
    static var alternateSelectedControlText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .alternateSelectedControlText)
#else
        .init()
#endif
    }

    /// The "amp&ersand" asset catalog color.
    static var ampErsand: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .ampErsand)
#else
        .init()
#endif
    }

    /// The "apostrophe\'s" asset catalog color.
    static var apostropheS: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .apostropheS)
#else
        .init()
#endif
    }

    #warning("The \"black\" color asset name resolves to a conflicting UIColor symbol \"black\". Try renaming the asset.")

    #warning("The \"blue\" color asset name resolves to a conflicting UIColor symbol \"blue\". Try renaming the asset.")

    /// The "brand-blue" asset catalog color.
    static var brandBlue: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .brandBlue)
#else
        .init()
#endif
    }

    /// The "brandRed2" asset catalog color.
    static var brandRed2: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .brandRed2)
#else
        .init()
#endif
    }

    /// The "brand_green" asset catalog color.
    static var brandGreen: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .brandGreen)
#else
        .init()
#endif
    }

    #warning("The \"brown\" color asset name resolves to a conflicting UIColor symbol \"brown\". Try renaming the asset.")

    /// The "camel-Case" asset catalog color.
    static var camelCase: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .camelCase)
#else
        .init()
#endif
    }

    /// The "class" asset catalog color.
    static var `class`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`class`)
#else
        .init()
#endif
    }

    #warning("The \"clear\" color asset name resolves to a conflicting UIColor symbol \"clear\". Try renaming the asset.")

    /// The "comma,name" asset catalog color.
    static var commaName: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .commaName)
#else
        .init()
#endif
    }

    /// The "control" asset catalog color.
    static var control: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .control)
#else
        .init()
#endif
    }

    /// The "controlAccent" asset catalog color.
    static var controlAccent: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlAccent)
#else
        .init()
#endif
    }

    /// The "controlBackground" asset catalog color.
    static var controlBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlBackground)
#else
        .init()
#endif
    }

    /// The "controlDarkShadow" asset catalog color.
    static var controlDarkShadow: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlDarkShadow)
#else
        .init()
#endif
    }

    /// The "controlHighlight" asset catalog color.
    static var controlHighlight: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlHighlight)
#else
        .init()
#endif
    }

    /// The "controlLightHighlight" asset catalog color.
    static var controlLightHighlight: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlLightHighlight)
#else
        .init()
#endif
    }

    /// The "controlShadow" asset catalog color.
    static var controlShadow: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlShadow)
#else
        .init()
#endif
    }

    /// The "controlText" asset catalog color.
    static var controlText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .controlText)
#else
        .init()
#endif
    }

    /// The "copy" asset catalog color.
    static var copy: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .copy)
#else
        .init()
#endif
    }

    #warning("The \"cyan\" color asset name resolves to a conflicting UIColor symbol \"cyan\". Try renaming the asset.")

    /// The "dark-mode" asset catalog resource namespace.
    enum DarkMode {

        /// The "dark-mode/Bg" asset catalog color.
        static var bg: UIKit.UIColor {
#if !os(watchOS)
            .init(resource: .DarkMode.bg)
#else
            .init()
#endif
        }

    }

    /// The "darkColor" asset catalog color.
    static var dark: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .dark)
#else
        .init()
#endif
    }

    #warning("The \"darkGray\" color asset name resolves to a conflicting UIColor symbol \"darkGray\". Try renaming the asset.")

    #warning("The \"darkText\" color asset name resolves to a conflicting UIColor symbol \"darkText\". Try renaming the asset.")

    /// The "default" asset catalog color.
    static var `default`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`default`)
#else
        .init()
#endif
    }

    /// The "defaultValue" asset catalog color.
    static var defaultValue: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .defaultValue)
#else
        .init()
#endif
    }

    /// The "description" asset catalog color.
    static var description: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .description)
#else
        .init()
#endif
    }

    /// The "disabledControlText" asset catalog color.
    static var disabledControlText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .disabledControlText)
#else
        .init()
#endif
    }

    /// The "emoji🎉" asset catalog color.
    static var emoji: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .emoji)
#else
        .init()
#endif
    }

    /// The "findHighlight" asset catalog color.
    static var findHighlight: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .findHighlight)
#else
        .init()
#endif
    }

    /// The "foo--bar" asset catalog color.
    static var fooBar: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .fooBar)
#else
        .init()
#endif
    }

    #warning("The \"green\" color asset name resolves to a conflicting UIColor symbol \"green\". Try renaming the asset.")

    /// The "grid" asset catalog color.
    static var grid: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .grid)
#else
        .init()
#endif
    }

    #warning("The \"groupTableViewBackground\" color asset name resolves to a conflicting UIColor symbol \"groupTableViewBackground\". Try renaming the asset.")

    /// The "header" asset catalog color.
    static var header: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .header)
#else
        .init()
#endif
    }

    /// The "headerText" asset catalog color.
    static var headerText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .headerText)
#else
        .init()
#endif
    }

    /// The "highlight" asset catalog color.
    static var highlight: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .highlight)
#else
        .init()
#endif
    }

    /// The "iOSColor" asset catalog color.
    static var iOS: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .iOS)
#else
        .init()
#endif
    }

    /// The "indigo" asset catalog color.
    static var indigo: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .indigo)
#else
        .init()
#endif
    }

    /// The "init" asset catalog color.
    static var `init`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`init`)
#else
        .init()
#endif
    }

    /// The "keyboardFocusIndicator" asset catalog color.
    static var keyboardFocusIndicator: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .keyboardFocusIndicator)
#else
        .init()
#endif
    }

    /// The "knob" asset catalog color.
    static var knob: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .knob)
#else
        .init()
#endif
    }

    #warning("The \"label\" color asset name resolves to a conflicting UIColor symbol \"label\". Try renaming the asset.")

    /// The "label2" asset catalog color.
    static var label2: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .label2)
#else
        .init()
#endif
    }

    #warning("The \"lightGray\" color asset name resolves to a conflicting UIColor symbol \"lightGray\". Try renaming the asset.")

    #warning("The \"lightText\" color asset name resolves to a conflicting UIColor symbol \"lightText\". Try renaming the asset.")

    #warning("The \"link\" color asset name resolves to a conflicting UIColor symbol \"link\". Try renaming the asset.")

    #warning("The \"magenta\" color asset name resolves to a conflicting UIColor symbol \"magenta\". Try renaming the asset.")

    /// The "mint" asset catalog color.
    static var mint: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .mint)
#else
        .init()
#endif
    }

    /// The "myURLColor" asset catalog color.
    static var myURL: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .myURL)
#else
        .init()
#endif
    }

    /// The "new" asset catalog color.
    static var new: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .new)
#else
        .init()
#endif
    }

    /// The "nil" asset catalog color.
    static var `nil`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`nil`)
#else
        .init()
#endif
    }

    #warning("The \"opaqueSeparator\" color asset name resolves to a conflicting UIColor symbol \"opaqueSeparator\". Try renaming the asset.")

    #warning("The \"orange\" color asset name resolves to a conflicting UIColor symbol \"orange\". Try renaming the asset.")

    /// The "paren(s)" asset catalog color.
    static var parenS: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .parenS)
#else
        .init()
#endif
    }

    /// The "pink" asset catalog color.
    static var pink: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .pink)
#else
        .init()
#endif
    }

    #warning("The \"placeholderText\" color asset name resolves to a conflicting UIColor symbol \"placeholderText\". Try renaming the asset.")

    /// The "plus+sign" asset catalog color.
    static var plusSign: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .plusSign)
#else
        .init()
#endif
    }

    /// The "primary" asset catalog color.
    static var primary: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .primary)
#else
        .init()
#endif
    }

    #warning("The \"purple\" color asset name resolves to a conflicting UIColor symbol \"purple\". Try renaming the asset.")

    #warning("The \"quaternaryLabel\" color asset name resolves to a conflicting UIColor symbol \"quaternaryLabel\". Try renaming the asset.")

    #warning("The \"quaternarySystemFill\" color asset name resolves to a conflicting UIColor symbol \"quaternarySystemFill\". Try renaming the asset.")

    /// The "quinaryLabel" asset catalog color.
    static var quinaryLabel: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .quinaryLabel)
#else
        .init()
#endif
    }

    /// The "quinarySystemFill" asset catalog color.
    static var quinarySystemFill: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .quinarySystemFill)
#else
        .init()
#endif
    }

    #warning("The \"red\" color asset name resolves to a conflicting UIColor symbol \"red\". Try renaming the asset.")

    /// The "red-500" asset catalog color.
    static var red500: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .red500)
#else
        .init()
#endif
    }

    /// The "scrollBar" asset catalog color.
    static var scrollBar: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .scrollBar)
#else
        .init()
#endif
    }

    /// The "scrollViewTexturedBackground" asset catalog color.
    static var scrollViewTexturedBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .scrollViewTexturedBackground)
#else
        .init()
#endif
    }

    /// The "scrubberTexturedBackground" asset catalog color.
    static var scrubberTexturedBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .scrubberTexturedBackground)
#else
        .init()
#endif
    }

    /// The "secondary" asset catalog color.
    static var secondary: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .secondary)
#else
        .init()
#endif
    }

    #warning("The \"secondaryLabel\" color asset name resolves to a conflicting UIColor symbol \"secondaryLabel\". Try renaming the asset.")

    /// The "secondarySelectedControl" asset catalog color.
    static var secondarySelectedControl: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .secondarySelectedControl)
#else
        .init()
#endif
    }

    #warning("The \"secondarySystemBackground\" color asset name resolves to a conflicting UIColor symbol \"secondarySystemBackground\". Try renaming the asset.")

    #warning("The \"secondarySystemFill\" color asset name resolves to a conflicting UIColor symbol \"secondarySystemFill\". Try renaming the asset.")

    #warning("The \"secondarySystemGroupedBackground\" color asset name resolves to a conflicting UIColor symbol \"secondarySystemGroupedBackground\". Try renaming the asset.")

    /// The "selectedContentBackground" asset catalog color.
    static var selectedContentBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedContentBackground)
#else
        .init()
#endif
    }

    /// The "selectedControl" asset catalog color.
    static var selectedControl: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedControl)
#else
        .init()
#endif
    }

    /// The "selectedControlText" asset catalog color.
    static var selectedControlText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedControlText)
#else
        .init()
#endif
    }

    /// The "selectedKnob" asset catalog color.
    static var selectedKnob: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedKnob)
#else
        .init()
#endif
    }

    /// The "selectedMenuItem" asset catalog color.
    static var selectedMenuItem: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedMenuItem)
#else
        .init()
#endif
    }

    /// The "selectedMenuItemText" asset catalog color.
    static var selectedMenuItemText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedMenuItemText)
#else
        .init()
#endif
    }

    /// The "selectedText" asset catalog color.
    static var selectedText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedText)
#else
        .init()
#endif
    }

    /// The "selectedTextBackground" asset catalog color.
    static var selectedTextBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .selectedTextBackground)
#else
        .init()
#endif
    }

    /// The "self" asset catalog color.
    static var `self`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`self`)
#else
        .init()
#endif
    }

    #warning("The \"separator\" color asset name resolves to a conflicting UIColor symbol \"separator\". Try renaming the asset.")

    /// The "shadow" asset catalog color.
    static var shadow: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .shadow)
#else
        .init()
#endif
    }

    /// The "superclass" asset catalog color.
    static var superclass: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .superclass)
#else
        .init()
#endif
    }

    #warning("The \"systemBackground\" color asset name resolves to a conflicting UIColor symbol \"systemBackground\". Try renaming the asset.")

    #warning("The \"systemBlue\" color asset name resolves to a conflicting UIColor symbol \"systemBlue\". Try renaming the asset.")

    #warning("The \"systemBrown\" color asset name resolves to a conflicting UIColor symbol \"systemBrown\". Try renaming the asset.")

    #warning("The \"systemCyan\" color asset name resolves to a conflicting UIColor symbol \"systemCyan\". Try renaming the asset.")

    #warning("The \"systemFill\" color asset name resolves to a conflicting UIColor symbol \"systemFill\". Try renaming the asset.")

    #warning("The \"systemGray\" color asset name resolves to a conflicting UIColor symbol \"systemGray\". Try renaming the asset.")

    #warning("The \"systemGray2\" color asset name resolves to a conflicting UIColor symbol \"systemGray2\". Try renaming the asset.")

    #warning("The \"systemGray3\" color asset name resolves to a conflicting UIColor symbol \"systemGray3\". Try renaming the asset.")

    #warning("The \"systemGray4\" color asset name resolves to a conflicting UIColor symbol \"systemGray4\". Try renaming the asset.")

    #warning("The \"systemGray5\" color asset name resolves to a conflicting UIColor symbol \"systemGray5\". Try renaming the asset.")

    #warning("The \"systemGray6\" color asset name resolves to a conflicting UIColor symbol \"systemGray6\". Try renaming the asset.")

    #warning("The \"systemGreen\" color asset name resolves to a conflicting UIColor symbol \"systemGreen\". Try renaming the asset.")

    #warning("The \"systemGroupedBackground\" color asset name resolves to a conflicting UIColor symbol \"systemGroupedBackground\". Try renaming the asset.")

    #warning("The \"systemIndigo\" color asset name resolves to a conflicting UIColor symbol \"systemIndigo\". Try renaming the asset.")

    #warning("The \"systemMint\" color asset name resolves to a conflicting UIColor symbol \"systemMint\". Try renaming the asset.")

    #warning("The \"systemOrange\" color asset name resolves to a conflicting UIColor symbol \"systemOrange\". Try renaming the asset.")

    #warning("The \"systemPink\" color asset name resolves to a conflicting UIColor symbol \"systemPink\". Try renaming the asset.")

    #warning("The \"systemPurple\" color asset name resolves to a conflicting UIColor symbol \"systemPurple\". Try renaming the asset.")

    #warning("The \"systemRed\" color asset name resolves to a conflicting UIColor symbol \"systemRed\". Try renaming the asset.")

    #warning("The \"systemTeal\" color asset name resolves to a conflicting UIColor symbol \"systemTeal\". Try renaming the asset.")

    #warning("The \"systemYellow\" color asset name resolves to a conflicting UIColor symbol \"systemYellow\". Try renaming the asset.")

    /// The "teal" asset catalog color.
    static var teal: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .teal)
#else
        .init()
#endif
    }

    #warning("The \"tertiaryLabel\" color asset name resolves to a conflicting UIColor symbol \"tertiaryLabel\". Try renaming the asset.")

    #warning("The \"tertiarySystemBackground\" color asset name resolves to a conflicting UIColor symbol \"tertiarySystemBackground\". Try renaming the asset.")

    #warning("The \"tertiarySystemFill\" color asset name resolves to a conflicting UIColor symbol \"tertiarySystemFill\". Try renaming the asset.")

    #warning("The \"tertiarySystemGroupedBackground\" color asset name resolves to a conflicting UIColor symbol \"tertiarySystemGroupedBackground\". Try renaming the asset.")

    /// The "text" asset catalog color.
    static var text: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .text)
#else
        .init()
#endif
    }

    /// The "textBackground" asset catalog color.
    static var textBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .textBackground)
#else
        .init()
#endif
    }

    /// The "textInsertionPoint" asset catalog color.
    static var textInsertionPoint: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .textInsertionPoint)
#else
        .init()
#endif
    }

    /// The "tint" asset catalog color.
    static var tint: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .tint)
#else
        .init()
#endif
    }

    /// The "true" asset catalog color.
    static var `true`: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .`true`)
#else
        .init()
#endif
    }

    /// The "underPageBackground" asset catalog color.
    static var underPageBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .underPageBackground)
#else
        .init()
#endif
    }

    /// The "unemphasizedSelectedContentBackground" asset catalog color.
    static var unemphasizedSelectedContentBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .unemphasizedSelectedContentBackground)
#else
        .init()
#endif
    }

    /// The "unemphasizedSelectedText" asset catalog color.
    static var unemphasizedSelectedText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .unemphasizedSelectedText)
#else
        .init()
#endif
    }

    /// The "unemphasizedSelectedTextBackground" asset catalog color.
    static var unemphasizedSelectedTextBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .unemphasizedSelectedTextBackground)
#else
        .init()
#endif
    }

    /// The "viewFlipsideBackground" asset catalog color.
    static var viewFlipsideBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .viewFlipsideBackground)
#else
        .init()
#endif
    }

    #warning("The \"white\" color asset name resolves to a conflicting UIColor symbol \"white\". Try renaming the asset.")

    /// The "windowBackground" asset catalog color.
    static var windowBackground: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .windowBackground)
#else
        .init()
#endif
    }

    /// The "windowFrame" asset catalog color.
    static var windowFrame: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .windowFrame)
#else
        .init()
#endif
    }

    /// The "windowFrameText" asset catalog color.
    static var windowFrameText: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .windowFrameText)
#else
        .init()
#endif
    }

    /// The "with.dots.many" asset catalog color.
    static var withDotsMany: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .withDotsMany)
#else
        .init()
#endif
    }

    /// The "x1y2" asset catalog color.
    static var x1Y2: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .x1Y2)
#else
        .init()
#endif
    }

    #warning("The \"yellow\" color asset name resolves to a conflicting UIColor symbol \"yellow\". Try renaming the asset.")

    /// The "zero" asset catalog color.
    static var zero: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .zero)
#else
        .init()
#endif
    }

    /// The "Ünïcode" asset catalog color.
    static var ünïcode: UIKit.UIColor {
#if !os(watchOS)
        .init(resource: .ünïcode)
#else
        .init()
#endif
    }

}
#endif

#if canImport(SwiftUI)
@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
extension SwiftUI.Color {

    /// The "#hash" asset catalog color.
    static var hash: SwiftUI.Color { .init(.hash) }

    /// The "1stColor" asset catalog color.
    static var _1St: SwiftUI.Color { .init(._1St) }

    /// The "9" asset catalog color.
    static var _9: SwiftUI.Color { .init(._9) }

    /// The "ABCdef" asset catalog color.
    static var abCdef: SwiftUI.Color { .init(.abCdef) }

    /// The "ALLCAPS" asset catalog color.
    static var ALLCAPS: SwiftUI.Color { .init(.ALLCAPS) }

    /// The "A_B" asset catalog color.
    static var A_B: SwiftUI.Color { .init(.A_B) }

    /// The "AccentColor" asset catalog color.
    static var accent: SwiftUI.Color { .init(.accent) }

    /// The "BothIdioms" asset catalog color.
    static var bothIdioms: SwiftUI.Color { .init(.bothIdioms) }

    /// The "Brand Yellow" asset catalog color.
    static var brandYellow: SwiftUI.Color { .init(.brandYellow) }

    /// The "BrandRed" asset catalog color.
    static var brandRed: SwiftUI.Color { .init(.brandRed) }

    /// The "Color" asset catalog color.
    static var color: SwiftUI.Color { .init(.color) }

    /// The "Color.Dark" asset catalog color.
    static var colorDark: SwiftUI.Color { .init(.colorDark) }

    #warning("The \"Gray\" color asset name resolves to a conflicting Color symbol \"gray\". Try renaming the asset.")

    /// The "Gray 100" asset catalog color.
    static var gray100: SwiftUI.Color { .init(.gray100) }

    /// The "HTML5Color" asset catalog color.
    static var HTML_5: SwiftUI.Color { .init(.HTML_5) }

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Tint" asset catalog color.
        static var tint: SwiftUI.Color { .init(.Icons.tint) }

    }

    /// The "PhoneOnly" asset catalog color.
    @available(watchOS, unavailable)
    static var phoneOnly: SwiftUI.Color? { .init(thinnableResource: .phoneOnly) }

    /// The "Protocol" asset catalog color.
    static var `protocol`: SwiftUI.Color { .init(.`protocol`) }

    /// The "Type" asset catalog color.
    static var type: SwiftUI.Color { .init(.type) }

    /// The "UPPER_snake" asset catalog color.
    static var upperSnake: SwiftUI.Color { .init(.upperSnake) }

    /// The "URLColor" asset catalog color.
    static var URL: SwiftUI.Color { .init(.URL) }

    /// The "UniPhone" asset catalog color.
    static var uniPhone: SwiftUI.Color { .init(.uniPhone) }

    /// The "_private" asset catalog color.
    static var `private`: SwiftUI.Color { .init(.`private`) }

    /// The "a" asset catalog color.
    static var a: SwiftUI.Color { .init(.a) }

    /// The "alloc" asset catalog color.
    static var alloc: SwiftUI.Color { .init(.alloc) }

    /// The "alternateSelectedControl" asset catalog color.
    static var alternateSelectedControl: SwiftUI.Color { .init(.alternateSelectedControl) }

    /// The "alternateSelectedControlText" asset catalog color.
    static var alternateSelectedControlText: SwiftUI.Color { .init(.alternateSelectedControlText) }

    /// The "amp&ersand" asset catalog color.
    static var ampErsand: SwiftUI.Color { .init(.ampErsand) }

    /// The "apostrophe\'s" asset catalog color.
    static var apostropheS: SwiftUI.Color { .init(.apostropheS) }

    #warning("The \"black\" color asset name resolves to a conflicting Color symbol \"black\". Try renaming the asset.")

    #warning("The \"blue\" color asset name resolves to a conflicting Color symbol \"blue\". Try renaming the asset.")

    /// The "brand-blue" asset catalog color.
    static var brandBlue: SwiftUI.Color { .init(.brandBlue) }

    /// The "brandRed2" asset catalog color.
    static var brandRed2: SwiftUI.Color { .init(.brandRed2) }

    /// The "brand_green" asset catalog color.
    static var brandGreen: SwiftUI.Color { .init(.brandGreen) }

    #warning("The \"brown\" color asset name resolves to a conflicting Color symbol \"brown\". Try renaming the asset.")

    /// The "camel-Case" asset catalog color.
    static var camelCase: SwiftUI.Color { .init(.camelCase) }

    /// The "class" asset catalog color.
    static var `class`: SwiftUI.Color { .init(.`class`) }

    #warning("The \"clear\" color asset name resolves to a conflicting Color symbol \"clear\". Try renaming the asset.")

    /// The "comma,name" asset catalog color.
    static var commaName: SwiftUI.Color { .init(.commaName) }

    /// The "control" asset catalog color.
    static var control: SwiftUI.Color { .init(.control) }

    /// The "controlAccent" asset catalog color.
    static var controlAccent: SwiftUI.Color { .init(.controlAccent) }

    /// The "controlBackground" asset catalog color.
    static var controlBackground: SwiftUI.Color { .init(.controlBackground) }

    /// The "controlDarkShadow" asset catalog color.
    static var controlDarkShadow: SwiftUI.Color { .init(.controlDarkShadow) }

    /// The "controlHighlight" asset catalog color.
    static var controlHighlight: SwiftUI.Color { .init(.controlHighlight) }

    /// The "controlLightHighlight" asset catalog color.
    static var controlLightHighlight: SwiftUI.Color { .init(.controlLightHighlight) }

    /// The "controlShadow" asset catalog color.
    static var controlShadow: SwiftUI.Color { .init(.controlShadow) }

    /// The "controlText" asset catalog color.
    static var controlText: SwiftUI.Color { .init(.controlText) }

    /// The "copy" asset catalog color.
    static var copy: SwiftUI.Color { .init(.copy) }

    #warning("The \"cyan\" color asset name resolves to a conflicting Color symbol \"cyan\". Try renaming the asset.")

    /// The "dark-mode" asset catalog resource namespace.
    enum DarkMode {

        /// The "dark-mode/Bg" asset catalog color.
        static var bg: SwiftUI.Color { .init(.DarkMode.bg) }

    }

    /// The "darkColor" asset catalog color.
    static var dark: SwiftUI.Color { .init(.dark) }

    /// The "darkGray" asset catalog color.
    static var darkGray: SwiftUI.Color { .init(.darkGray) }

    /// The "darkText" asset catalog color.
    static var darkText: SwiftUI.Color { .init(.darkText) }

    /// The "default" asset catalog color.
    static var `default`: SwiftUI.Color { .init(.`default`) }

    /// The "defaultValue" asset catalog color.
    static var defaultValue: SwiftUI.Color { .init(.defaultValue) }

    /// The "description" asset catalog color.
    static var description: SwiftUI.Color { .init(.description) }

    /// The "disabledControlText" asset catalog color.
    static var disabledControlText: SwiftUI.Color { .init(.disabledControlText) }

    /// The "emoji🎉" asset catalog color.
    static var emoji: SwiftUI.Color { .init(.emoji) }

    /// The "findHighlight" asset catalog color.
    static var findHighlight: SwiftUI.Color { .init(.findHighlight) }

    /// The "foo--bar" asset catalog color.
    static var fooBar: SwiftUI.Color { .init(.fooBar) }

    #warning("The \"green\" color asset name resolves to a conflicting Color symbol \"green\". Try renaming the asset.")

    /// The "grid" asset catalog color.
    static var grid: SwiftUI.Color { .init(.grid) }

    /// The "groupTableViewBackground" asset catalog color.
    static var groupTableViewBackground: SwiftUI.Color { .init(.groupTableViewBackground) }

    /// The "header" asset catalog color.
    static var header: SwiftUI.Color { .init(.header) }

    /// The "headerText" asset catalog color.
    static var headerText: SwiftUI.Color { .init(.headerText) }

    /// The "highlight" asset catalog color.
    static var highlight: SwiftUI.Color { .init(.highlight) }

    /// The "iOSColor" asset catalog color.
    static var iOS: SwiftUI.Color { .init(.iOS) }

    #warning("The \"indigo\" color asset name resolves to a conflicting Color symbol \"indigo\". Try renaming the asset.")

    /// The "init" asset catalog color.
    static var `init`: SwiftUI.Color { .init(.`init`) }

    /// The "keyboardFocusIndicator" asset catalog color.
    static var keyboardFocusIndicator: SwiftUI.Color { .init(.keyboardFocusIndicator) }

    /// The "knob" asset catalog color.
    static var knob: SwiftUI.Color { .init(.knob) }

    /// The "label" asset catalog color.
    static var label: SwiftUI.Color { .init(.label) }

    /// The "label2" asset catalog color.
    static var label2: SwiftUI.Color { .init(.label2) }

    /// The "lightGray" asset catalog color.
    static var lightGray: SwiftUI.Color { .init(.lightGray) }

    /// The "lightText" asset catalog color.
    static var lightText: SwiftUI.Color { .init(.lightText) }

    /// The "link" asset catalog color.
    static var link: SwiftUI.Color { .init(.link) }

    /// The "magenta" asset catalog color.
    static var magenta: SwiftUI.Color { .init(.magenta) }

    #warning("The \"mint\" color asset name resolves to a conflicting Color symbol \"mint\". Try renaming the asset.")

    /// The "myURLColor" asset catalog color.
    static var myURL: SwiftUI.Color { .init(.myURL) }

    /// The "new" asset catalog color.
    static var new: SwiftUI.Color { .init(.new) }

    /// The "nil" asset catalog color.
    static var `nil`: SwiftUI.Color { .init(.`nil`) }

    /// The "opaqueSeparator" asset catalog color.
    static var opaqueSeparator: SwiftUI.Color { .init(.opaqueSeparator) }

    #warning("The \"orange\" color asset name resolves to a conflicting Color symbol \"orange\". Try renaming the asset.")

    /// The "paren(s)" asset catalog color.
    static var parenS: SwiftUI.Color { .init(.parenS) }

    #warning("The \"pink\" color asset name resolves to a conflicting Color symbol \"pink\". Try renaming the asset.")

    /// The "placeholderText" asset catalog color.
    static var placeholderText: SwiftUI.Color { .init(.placeholderText) }

    /// The "plus+sign" asset catalog color.
    static var plusSign: SwiftUI.Color { .init(.plusSign) }

    #warning("The \"primary\" color asset name resolves to a conflicting Color symbol \"primary\". Try renaming the asset.")

    #warning("The \"purple\" color asset name resolves to a conflicting Color symbol \"purple\". Try renaming the asset.")

    /// The "quaternaryLabel" asset catalog color.
    static var quaternaryLabel: SwiftUI.Color { .init(.quaternaryLabel) }

    /// The "quaternarySystemFill" asset catalog color.
    static var quaternarySystemFill: SwiftUI.Color { .init(.quaternarySystemFill) }

    /// The "quinaryLabel" asset catalog color.
    static var quinaryLabel: SwiftUI.Color { .init(.quinaryLabel) }

    /// The "quinarySystemFill" asset catalog color.
    static var quinarySystemFill: SwiftUI.Color { .init(.quinarySystemFill) }

    #warning("The \"red\" color asset name resolves to a conflicting Color symbol \"red\". Try renaming the asset.")

    /// The "red-500" asset catalog color.
    static var red500: SwiftUI.Color { .init(.red500) }

    /// The "scrollBar" asset catalog color.
    static var scrollBar: SwiftUI.Color { .init(.scrollBar) }

    /// The "scrollViewTexturedBackground" asset catalog color.
    static var scrollViewTexturedBackground: SwiftUI.Color { .init(.scrollViewTexturedBackground) }

    /// The "scrubberTexturedBackground" asset catalog color.
    static var scrubberTexturedBackground: SwiftUI.Color { .init(.scrubberTexturedBackground) }

    #warning("The \"secondary\" color asset name resolves to a conflicting Color symbol \"secondary\". Try renaming the asset.")

    /// The "secondaryLabel" asset catalog color.
    static var secondaryLabel: SwiftUI.Color { .init(.secondaryLabel) }

    /// The "secondarySelectedControl" asset catalog color.
    static var secondarySelectedControl: SwiftUI.Color { .init(.secondarySelectedControl) }

    /// The "secondarySystemBackground" asset catalog color.
    static var secondarySystemBackground: SwiftUI.Color { .init(.secondarySystemBackground) }

    /// The "secondarySystemFill" asset catalog color.
    static var secondarySystemFill: SwiftUI.Color { .init(.secondarySystemFill) }

    /// The "secondarySystemGroupedBackground" asset catalog color.
    static var secondarySystemGroupedBackground: SwiftUI.Color { .init(.secondarySystemGroupedBackground) }

    /// The "selectedContentBackground" asset catalog color.
    static var selectedContentBackground: SwiftUI.Color { .init(.selectedContentBackground) }

    /// The "selectedControl" asset catalog color.
    static var selectedControl: SwiftUI.Color { .init(.selectedControl) }

    /// The "selectedControlText" asset catalog color.
    static var selectedControlText: SwiftUI.Color { .init(.selectedControlText) }

    /// The "selectedKnob" asset catalog color.
    static var selectedKnob: SwiftUI.Color { .init(.selectedKnob) }

    /// The "selectedMenuItem" asset catalog color.
    static var selectedMenuItem: SwiftUI.Color { .init(.selectedMenuItem) }

    /// The "selectedMenuItemText" asset catalog color.
    static var selectedMenuItemText: SwiftUI.Color { .init(.selectedMenuItemText) }

    /// The "selectedText" asset catalog color.
    static var selectedText: SwiftUI.Color { .init(.selectedText) }

    /// The "selectedTextBackground" asset catalog color.
    static var selectedTextBackground: SwiftUI.Color { .init(.selectedTextBackground) }

    /// The "self" asset catalog color.
    static var `self`: SwiftUI.Color { .init(.`self`) }

    /// The "separator" asset catalog color.
    static var separator: SwiftUI.Color { .init(.separator) }

    /// The "shadow" asset catalog color.
    static var shadow: SwiftUI.Color { .init(.shadow) }

    /// The "superclass" asset catalog color.
    static var superclass: SwiftUI.Color { .init(.superclass) }

    /// The "systemBackground" asset catalog color.
    static var systemBackground: SwiftUI.Color { .init(.systemBackground) }

    /// The "systemBlue" asset catalog color.
    static var systemBlue: SwiftUI.Color { .init(.systemBlue) }

    /// The "systemBrown" asset catalog color.
    static var systemBrown: SwiftUI.Color { .init(.systemBrown) }

    /// The "systemCyan" asset catalog color.
    static var systemCyan: SwiftUI.Color { .init(.systemCyan) }

    /// The "systemFill" asset catalog color.
    static var systemFill: SwiftUI.Color { .init(.systemFill) }

    /// The "systemGray" asset catalog color.
    static var systemGray: SwiftUI.Color { .init(.systemGray) }

    /// The "systemGray2" asset catalog color.
    static var systemGray2: SwiftUI.Color { .init(.systemGray2) }

    /// The "systemGray3" asset catalog color.
    static var systemGray3: SwiftUI.Color { .init(.systemGray3) }

    /// The "systemGray4" asset catalog color.
    static var systemGray4: SwiftUI.Color { .init(.systemGray4) }

    /// The "systemGray5" asset catalog color.
    static var systemGray5: SwiftUI.Color { .init(.systemGray5) }

    /// The "systemGray6" asset catalog color.
    static var systemGray6: SwiftUI.Color { .init(.systemGray6) }

    /// The "systemGreen" asset catalog color.
    static var systemGreen: SwiftUI.Color { .init(.systemGreen) }

    /// The "systemGroupedBackground" asset catalog color.
    static var systemGroupedBackground: SwiftUI.Color { .init(.systemGroupedBackground) }

    /// The "systemIndigo" asset catalog color.
    static var systemIndigo: SwiftUI.Color { .init(.systemIndigo) }

    /// The "systemMint" asset catalog color.
    static var systemMint: SwiftUI.Color { .init(.systemMint) }

    /// The "systemOrange" asset catalog color.
    static var systemOrange: SwiftUI.Color { .init(.systemOrange) }

    /// The "systemPink" asset catalog color.
    static var systemPink: SwiftUI.Color { .init(.systemPink) }

    /// The "systemPurple" asset catalog color.
    static var systemPurple: SwiftUI.Color { .init(.systemPurple) }

    /// The "systemRed" asset catalog color.
    static var systemRed: SwiftUI.Color { .init(.systemRed) }

    /// The "systemTeal" asset catalog color.
    static var systemTeal: SwiftUI.Color { .init(.systemTeal) }

    /// The "systemYellow" asset catalog color.
    static var systemYellow: SwiftUI.Color { .init(.systemYellow) }

    #warning("The \"teal\" color asset name resolves to a conflicting Color symbol \"teal\". Try renaming the asset.")

    /// The "tertiaryLabel" asset catalog color.
    static var tertiaryLabel: SwiftUI.Color { .init(.tertiaryLabel) }

    /// The "tertiarySystemBackground" asset catalog color.
    static var tertiarySystemBackground: SwiftUI.Color { .init(.tertiarySystemBackground) }

    /// The "tertiarySystemFill" asset catalog color.
    static var tertiarySystemFill: SwiftUI.Color { .init(.tertiarySystemFill) }

    /// The "tertiarySystemGroupedBackground" asset catalog color.
    static var tertiarySystemGroupedBackground: SwiftUI.Color { .init(.tertiarySystemGroupedBackground) }

    /// The "text" asset catalog color.
    static var text: SwiftUI.Color { .init(.text) }

    /// The "textBackground" asset catalog color.
    static var textBackground: SwiftUI.Color { .init(.textBackground) }

    /// The "textInsertionPoint" asset catalog color.
    static var textInsertionPoint: SwiftUI.Color { .init(.textInsertionPoint) }

    /// The "tint" asset catalog color.
    static var tint: SwiftUI.Color { .init(.tint) }

    /// The "true" asset catalog color.
    static var `true`: SwiftUI.Color { .init(.`true`) }

    /// The "underPageBackground" asset catalog color.
    static var underPageBackground: SwiftUI.Color { .init(.underPageBackground) }

    /// The "unemphasizedSelectedContentBackground" asset catalog color.
    static var unemphasizedSelectedContentBackground: SwiftUI.Color { .init(.unemphasizedSelectedContentBackground) }

    /// The "unemphasizedSelectedText" asset catalog color.
    static var unemphasizedSelectedText: SwiftUI.Color { .init(.unemphasizedSelectedText) }

    /// The "unemphasizedSelectedTextBackground" asset catalog color.
    static var unemphasizedSelectedTextBackground: SwiftUI.Color { .init(.unemphasizedSelectedTextBackground) }

    /// The "viewFlipsideBackground" asset catalog color.
    static var viewFlipsideBackground: SwiftUI.Color { .init(.viewFlipsideBackground) }

    #warning("The \"white\" color asset name resolves to a conflicting Color symbol \"white\". Try renaming the asset.")

    /// The "windowBackground" asset catalog color.
    static var windowBackground: SwiftUI.Color { .init(.windowBackground) }

    /// The "windowFrame" asset catalog color.
    static var windowFrame: SwiftUI.Color { .init(.windowFrame) }

    /// The "windowFrameText" asset catalog color.
    static var windowFrameText: SwiftUI.Color { .init(.windowFrameText) }

    /// The "with.dots.many" asset catalog color.
    static var withDotsMany: SwiftUI.Color { .init(.withDotsMany) }

    /// The "x1y2" asset catalog color.
    static var x1Y2: SwiftUI.Color { .init(.x1Y2) }

    #warning("The \"yellow\" color asset name resolves to a conflicting Color symbol \"yellow\". Try renaming the asset.")

    /// The "zero" asset catalog color.
    static var zero: SwiftUI.Color { .init(.zero) }

    /// The "Ünïcode" asset catalog color.
    static var ünïcode: SwiftUI.Color { .init(.ünïcode) }

}

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
extension SwiftUI.ShapeStyle where Self == SwiftUI.Color {

    /// The "#hash" asset catalog color.
    static var hash: SwiftUI.Color { .init(.hash) }

    /// The "1stColor" asset catalog color.
    static var _1St: SwiftUI.Color { .init(._1St) }

    /// The "9" asset catalog color.
    static var _9: SwiftUI.Color { .init(._9) }

    /// The "ABCdef" asset catalog color.
    static var abCdef: SwiftUI.Color { .init(.abCdef) }

    /// The "ALLCAPS" asset catalog color.
    static var ALLCAPS: SwiftUI.Color { .init(.ALLCAPS) }

    /// The "A_B" asset catalog color.
    static var A_B: SwiftUI.Color { .init(.A_B) }

    /// The "AccentColor" asset catalog color.
    static var accent: SwiftUI.Color { .init(.accent) }

    /// The "BothIdioms" asset catalog color.
    static var bothIdioms: SwiftUI.Color { .init(.bothIdioms) }

    /// The "Brand Yellow" asset catalog color.
    static var brandYellow: SwiftUI.Color { .init(.brandYellow) }

    /// The "BrandRed" asset catalog color.
    static var brandRed: SwiftUI.Color { .init(.brandRed) }

    /// The "Color" asset catalog color.
    static var color: SwiftUI.Color { .init(.color) }

    /// The "Color.Dark" asset catalog color.
    static var colorDark: SwiftUI.Color { .init(.colorDark) }

    /// The "Gray 100" asset catalog color.
    static var gray100: SwiftUI.Color { .init(.gray100) }

    /// The "HTML5Color" asset catalog color.
    static var HTML_5: SwiftUI.Color { .init(.HTML_5) }

    /// The "PhoneOnly" asset catalog color.
    @available(watchOS, unavailable)
    static var phoneOnly: SwiftUI.Color? { .init(thinnableResource: .phoneOnly) }

    /// The "Protocol" asset catalog color.
    static var `protocol`: SwiftUI.Color { .init(.`protocol`) }

    /// The "Type" asset catalog color.
    static var type: SwiftUI.Color { .init(.type) }

    /// The "UPPER_snake" asset catalog color.
    static var upperSnake: SwiftUI.Color { .init(.upperSnake) }

    /// The "URLColor" asset catalog color.
    static var URL: SwiftUI.Color { .init(.URL) }

    /// The "UniPhone" asset catalog color.
    static var uniPhone: SwiftUI.Color { .init(.uniPhone) }

    /// The "_private" asset catalog color.
    static var `private`: SwiftUI.Color { .init(.`private`) }

    /// The "a" asset catalog color.
    static var a: SwiftUI.Color { .init(.a) }

    /// The "alloc" asset catalog color.
    static var alloc: SwiftUI.Color { .init(.alloc) }

    /// The "alternateSelectedControl" asset catalog color.
    static var alternateSelectedControl: SwiftUI.Color { .init(.alternateSelectedControl) }

    /// The "alternateSelectedControlText" asset catalog color.
    static var alternateSelectedControlText: SwiftUI.Color { .init(.alternateSelectedControlText) }

    /// The "amp&ersand" asset catalog color.
    static var ampErsand: SwiftUI.Color { .init(.ampErsand) }

    /// The "apostrophe\'s" asset catalog color.
    static var apostropheS: SwiftUI.Color { .init(.apostropheS) }

    /// The "brand-blue" asset catalog color.
    static var brandBlue: SwiftUI.Color { .init(.brandBlue) }

    /// The "brandRed2" asset catalog color.
    static var brandRed2: SwiftUI.Color { .init(.brandRed2) }

    /// The "brand_green" asset catalog color.
    static var brandGreen: SwiftUI.Color { .init(.brandGreen) }

    /// The "camel-Case" asset catalog color.
    static var camelCase: SwiftUI.Color { .init(.camelCase) }

    /// The "class" asset catalog color.
    static var `class`: SwiftUI.Color { .init(.`class`) }

    /// The "comma,name" asset catalog color.
    static var commaName: SwiftUI.Color { .init(.commaName) }

    /// The "control" asset catalog color.
    static var control: SwiftUI.Color { .init(.control) }

    /// The "controlAccent" asset catalog color.
    static var controlAccent: SwiftUI.Color { .init(.controlAccent) }

    /// The "controlBackground" asset catalog color.
    static var controlBackground: SwiftUI.Color { .init(.controlBackground) }

    /// The "controlDarkShadow" asset catalog color.
    static var controlDarkShadow: SwiftUI.Color { .init(.controlDarkShadow) }

    /// The "controlHighlight" asset catalog color.
    static var controlHighlight: SwiftUI.Color { .init(.controlHighlight) }

    /// The "controlLightHighlight" asset catalog color.
    static var controlLightHighlight: SwiftUI.Color { .init(.controlLightHighlight) }

    /// The "controlShadow" asset catalog color.
    static var controlShadow: SwiftUI.Color { .init(.controlShadow) }

    /// The "controlText" asset catalog color.
    static var controlText: SwiftUI.Color { .init(.controlText) }

    /// The "copy" asset catalog color.
    static var copy: SwiftUI.Color { .init(.copy) }

    /// The "darkColor" asset catalog color.
    static var dark: SwiftUI.Color { .init(.dark) }

    /// The "darkGray" asset catalog color.
    static var darkGray: SwiftUI.Color { .init(.darkGray) }

    /// The "darkText" asset catalog color.
    static var darkText: SwiftUI.Color { .init(.darkText) }

    /// The "default" asset catalog color.
    static var `default`: SwiftUI.Color { .init(.`default`) }

    /// The "defaultValue" asset catalog color.
    static var defaultValue: SwiftUI.Color { .init(.defaultValue) }

    /// The "description" asset catalog color.
    static var description: SwiftUI.Color { .init(.description) }

    /// The "disabledControlText" asset catalog color.
    static var disabledControlText: SwiftUI.Color { .init(.disabledControlText) }

    /// The "emoji🎉" asset catalog color.
    static var emoji: SwiftUI.Color { .init(.emoji) }

    /// The "findHighlight" asset catalog color.
    static var findHighlight: SwiftUI.Color { .init(.findHighlight) }

    /// The "foo--bar" asset catalog color.
    static var fooBar: SwiftUI.Color { .init(.fooBar) }

    /// The "grid" asset catalog color.
    static var grid: SwiftUI.Color { .init(.grid) }

    /// The "groupTableViewBackground" asset catalog color.
    static var groupTableViewBackground: SwiftUI.Color { .init(.groupTableViewBackground) }

    /// The "header" asset catalog color.
    static var header: SwiftUI.Color { .init(.header) }

    /// The "headerText" asset catalog color.
    static var headerText: SwiftUI.Color { .init(.headerText) }

    /// The "highlight" asset catalog color.
    static var highlight: SwiftUI.Color { .init(.highlight) }

    /// The "iOSColor" asset catalog color.
    static var iOS: SwiftUI.Color { .init(.iOS) }

    /// The "init" asset catalog color.
    static var `init`: SwiftUI.Color { .init(.`init`) }

    /// The "keyboardFocusIndicator" asset catalog color.
    static var keyboardFocusIndicator: SwiftUI.Color { .init(.keyboardFocusIndicator) }

    /// The "knob" asset catalog color.
    static var knob: SwiftUI.Color { .init(.knob) }

    /// The "label" asset catalog color.
    static var label: SwiftUI.Color { .init(.label) }

    /// The "label2" asset catalog color.
    static var label2: SwiftUI.Color { .init(.label2) }

    /// The "lightGray" asset catalog color.
    static var lightGray: SwiftUI.Color { .init(.lightGray) }

    /// The "lightText" asset catalog color.
    static var lightText: SwiftUI.Color { .init(.lightText) }

    /// The "link" asset catalog color.
    static var link: SwiftUI.Color { .init(.link) }

    /// The "magenta" asset catalog color.
    static var magenta: SwiftUI.Color { .init(.magenta) }

    /// The "myURLColor" asset catalog color.
    static var myURL: SwiftUI.Color { .init(.myURL) }

    /// The "new" asset catalog color.
    static var new: SwiftUI.Color { .init(.new) }

    /// The "nil" asset catalog color.
    static var `nil`: SwiftUI.Color { .init(.`nil`) }

    /// The "opaqueSeparator" asset catalog color.
    static var opaqueSeparator: SwiftUI.Color { .init(.opaqueSeparator) }

    /// The "paren(s)" asset catalog color.
    static var parenS: SwiftUI.Color { .init(.parenS) }

    /// The "placeholderText" asset catalog color.
    static var placeholderText: SwiftUI.Color { .init(.placeholderText) }

    /// The "plus+sign" asset catalog color.
    static var plusSign: SwiftUI.Color { .init(.plusSign) }

    /// The "quaternaryLabel" asset catalog color.
    static var quaternaryLabel: SwiftUI.Color { .init(.quaternaryLabel) }

    /// The "quaternarySystemFill" asset catalog color.
    static var quaternarySystemFill: SwiftUI.Color { .init(.quaternarySystemFill) }

    /// The "quinaryLabel" asset catalog color.
    static var quinaryLabel: SwiftUI.Color { .init(.quinaryLabel) }

    /// The "quinarySystemFill" asset catalog color.
    static var quinarySystemFill: SwiftUI.Color { .init(.quinarySystemFill) }

    /// The "red-500" asset catalog color.
    static var red500: SwiftUI.Color { .init(.red500) }

    /// The "scrollBar" asset catalog color.
    static var scrollBar: SwiftUI.Color { .init(.scrollBar) }

    /// The "scrollViewTexturedBackground" asset catalog color.
    static var scrollViewTexturedBackground: SwiftUI.Color { .init(.scrollViewTexturedBackground) }

    /// The "scrubberTexturedBackground" asset catalog color.
    static var scrubberTexturedBackground: SwiftUI.Color { .init(.scrubberTexturedBackground) }

    /// The "secondaryLabel" asset catalog color.
    static var secondaryLabel: SwiftUI.Color { .init(.secondaryLabel) }

    /// The "secondarySelectedControl" asset catalog color.
    static var secondarySelectedControl: SwiftUI.Color { .init(.secondarySelectedControl) }

    /// The "secondarySystemBackground" asset catalog color.
    static var secondarySystemBackground: SwiftUI.Color { .init(.secondarySystemBackground) }

    /// The "secondarySystemFill" asset catalog color.
    static var secondarySystemFill: SwiftUI.Color { .init(.secondarySystemFill) }

    /// The "secondarySystemGroupedBackground" asset catalog color.
    static var secondarySystemGroupedBackground: SwiftUI.Color { .init(.secondarySystemGroupedBackground) }

    /// The "selectedContentBackground" asset catalog color.
    static var selectedContentBackground: SwiftUI.Color { .init(.selectedContentBackground) }

    /// The "selectedControl" asset catalog color.
    static var selectedControl: SwiftUI.Color { .init(.selectedControl) }

    /// The "selectedControlText" asset catalog color.
    static var selectedControlText: SwiftUI.Color { .init(.selectedControlText) }

    /// The "selectedKnob" asset catalog color.
    static var selectedKnob: SwiftUI.Color { .init(.selectedKnob) }

    /// The "selectedMenuItem" asset catalog color.
    static var selectedMenuItem: SwiftUI.Color { .init(.selectedMenuItem) }

    /// The "selectedMenuItemText" asset catalog color.
    static var selectedMenuItemText: SwiftUI.Color { .init(.selectedMenuItemText) }

    /// The "selectedText" asset catalog color.
    static var selectedText: SwiftUI.Color { .init(.selectedText) }

    /// The "selectedTextBackground" asset catalog color.
    static var selectedTextBackground: SwiftUI.Color { .init(.selectedTextBackground) }

    /// The "self" asset catalog color.
    static var `self`: SwiftUI.Color { .init(.`self`) }

    /// The "separator" asset catalog color.
    static var separator: SwiftUI.Color { .init(.separator) }

    /// The "shadow" asset catalog color.
    static var shadow: SwiftUI.Color { .init(.shadow) }

    /// The "superclass" asset catalog color.
    static var superclass: SwiftUI.Color { .init(.superclass) }

    /// The "systemBackground" asset catalog color.
    static var systemBackground: SwiftUI.Color { .init(.systemBackground) }

    /// The "systemBlue" asset catalog color.
    static var systemBlue: SwiftUI.Color { .init(.systemBlue) }

    /// The "systemBrown" asset catalog color.
    static var systemBrown: SwiftUI.Color { .init(.systemBrown) }

    /// The "systemCyan" asset catalog color.
    static var systemCyan: SwiftUI.Color { .init(.systemCyan) }

    /// The "systemFill" asset catalog color.
    static var systemFill: SwiftUI.Color { .init(.systemFill) }

    /// The "systemGray" asset catalog color.
    static var systemGray: SwiftUI.Color { .init(.systemGray) }

    /// The "systemGray2" asset catalog color.
    static var systemGray2: SwiftUI.Color { .init(.systemGray2) }

    /// The "systemGray3" asset catalog color.
    static var systemGray3: SwiftUI.Color { .init(.systemGray3) }

    /// The "systemGray4" asset catalog color.
    static var systemGray4: SwiftUI.Color { .init(.systemGray4) }

    /// The "systemGray5" asset catalog color.
    static var systemGray5: SwiftUI.Color { .init(.systemGray5) }

    /// The "systemGray6" asset catalog color.
    static var systemGray6: SwiftUI.Color { .init(.systemGray6) }

    /// The "systemGreen" asset catalog color.
    static var systemGreen: SwiftUI.Color { .init(.systemGreen) }

    /// The "systemGroupedBackground" asset catalog color.
    static var systemGroupedBackground: SwiftUI.Color { .init(.systemGroupedBackground) }

    /// The "systemIndigo" asset catalog color.
    static var systemIndigo: SwiftUI.Color { .init(.systemIndigo) }

    /// The "systemMint" asset catalog color.
    static var systemMint: SwiftUI.Color { .init(.systemMint) }

    /// The "systemOrange" asset catalog color.
    static var systemOrange: SwiftUI.Color { .init(.systemOrange) }

    /// The "systemPink" asset catalog color.
    static var systemPink: SwiftUI.Color { .init(.systemPink) }

    /// The "systemPurple" asset catalog color.
    static var systemPurple: SwiftUI.Color { .init(.systemPurple) }

    /// The "systemRed" asset catalog color.
    static var systemRed: SwiftUI.Color { .init(.systemRed) }

    /// The "systemTeal" asset catalog color.
    static var systemTeal: SwiftUI.Color { .init(.systemTeal) }

    /// The "systemYellow" asset catalog color.
    static var systemYellow: SwiftUI.Color { .init(.systemYellow) }

    /// The "tertiaryLabel" asset catalog color.
    static var tertiaryLabel: SwiftUI.Color { .init(.tertiaryLabel) }

    /// The "tertiarySystemBackground" asset catalog color.
    static var tertiarySystemBackground: SwiftUI.Color { .init(.tertiarySystemBackground) }

    /// The "tertiarySystemFill" asset catalog color.
    static var tertiarySystemFill: SwiftUI.Color { .init(.tertiarySystemFill) }

    /// The "tertiarySystemGroupedBackground" asset catalog color.
    static var tertiarySystemGroupedBackground: SwiftUI.Color { .init(.tertiarySystemGroupedBackground) }

    /// The "text" asset catalog color.
    static var text: SwiftUI.Color { .init(.text) }

    /// The "textBackground" asset catalog color.
    static var textBackground: SwiftUI.Color { .init(.textBackground) }

    /// The "textInsertionPoint" asset catalog color.
    static var textInsertionPoint: SwiftUI.Color { .init(.textInsertionPoint) }

    /// The "tint" asset catalog color.
    static var tint: SwiftUI.Color { .init(.tint) }

    /// The "true" asset catalog color.
    static var `true`: SwiftUI.Color { .init(.`true`) }

    /// The "underPageBackground" asset catalog color.
    static var underPageBackground: SwiftUI.Color { .init(.underPageBackground) }

    /// The "unemphasizedSelectedContentBackground" asset catalog color.
    static var unemphasizedSelectedContentBackground: SwiftUI.Color { .init(.unemphasizedSelectedContentBackground) }

    /// The "unemphasizedSelectedText" asset catalog color.
    static var unemphasizedSelectedText: SwiftUI.Color { .init(.unemphasizedSelectedText) }

    /// The "unemphasizedSelectedTextBackground" asset catalog color.
    static var unemphasizedSelectedTextBackground: SwiftUI.Color { .init(.unemphasizedSelectedTextBackground) }

    /// The "viewFlipsideBackground" asset catalog color.
    static var viewFlipsideBackground: SwiftUI.Color { .init(.viewFlipsideBackground) }

    /// The "windowBackground" asset catalog color.
    static var windowBackground: SwiftUI.Color { .init(.windowBackground) }

    /// The "windowFrame" asset catalog color.
    static var windowFrame: SwiftUI.Color { .init(.windowFrame) }

    /// The "windowFrameText" asset catalog color.
    static var windowFrameText: SwiftUI.Color { .init(.windowFrameText) }

    /// The "with.dots.many" asset catalog color.
    static var withDotsMany: SwiftUI.Color { .init(.withDotsMany) }

    /// The "x1y2" asset catalog color.
    static var x1Y2: SwiftUI.Color { .init(.x1Y2) }

    /// The "zero" asset catalog color.
    static var zero: SwiftUI.Color { .init(.zero) }

    /// The "Ünïcode" asset catalog color.
    static var ünïcode: SwiftUI.Color { .init(.ünïcode) }

}
#endif

// MARK: - Image Symbol Extensions -

#if canImport(AppKit)
@available(macOS 14.0, *)
@available(macCatalyst, unavailable)
extension AppKit.NSImage {

    /// The "1stImage" asset catalog image.
    static var _1St: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: ._1St)
#else
        .init()
#endif
    }

    /// The "BigImageView" asset catalog image.
    static var bigImageView: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .bigImageView)
#else
        .init()
#endif
    }

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Star" asset catalog image.
        static var star: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
            .init(resource: .Icons.star)
#else
            .init()
#endif
        }

    }

    /// The "Image" asset catalog image.
    static var image: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .image)
#else
        .init()
#endif
    }

    /// The "Image Name" asset catalog image.
    static var imageName: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .imageName)
#else
        .init()
#endif
    }

    /// The "Image2" asset catalog image.
    static var image2: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .image2)
#else
        .init()
#endif
    }

    /// The "Logo" asset catalog image.
    static var logo: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .logo)
#else
        .init()
#endif
    }

    /// The "Nested" asset catalog resource namespace.
    enum Nested {

        /// The "Nested/Deep" asset catalog resource namespace.
        enum Deep {

            /// The "Nested/Deep/Leaf" asset catalog image.
            static var leaf: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
                .init(resource: .Nested.Deep.leaf)
#else
                .init()
#endif
            }

        }

    }

    /// The "PadOnly" asset catalog image.
    static var padOnly: AppKit.NSImage? {
#if !targetEnvironment(macCatalyst)
        .init(thinnableResource: .padOnly)
#else
        .init()
#endif
    }

    /// The "actions" asset catalog image.
    static var actions: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .actions)
#else
        .init()
#endif
    }

    /// The "add" asset catalog image.
    static var add: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .add)
#else
        .init()
#endif
    }

    /// The "branch" asset catalog image.
    static var branch: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .branch)
#else
        .init()
#endif
    }

    /// The "checkmark" asset catalog image.
    static var checkmark: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .checkmark)
#else
        .init()
#endif
    }

    /// The "highlightedBranch" asset catalog image.
    static var highlightedBranch: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .highlightedBranch)
#else
        .init()
#endif
    }

    /// The "init" asset catalog image.
    static var `init`: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`init`)
#else
        .init()
#endif
    }

    /// The "logo-dark" asset catalog image.
    static var logoDark: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .logoDark)
#else
        .init()
#endif
    }

    /// The "my icons" asset catalog resource namespace.
    enum MyIcons {

        /// The "my icons/Home" asset catalog image.
        static var home: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
            .init(resource: .MyIcons.home)
#else
            .init()
#endif
        }

    }

    /// The "photo" asset catalog image.
    static var photo: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .photo)
#else
        .init()
#endif
    }

    /// The "remove" asset catalog image.
    static var remove: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .remove)
#else
        .init()
#endif
    }

    /// The "self" asset catalog image.
    static var `self`: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .`self`)
#else
        .init()
#endif
    }

    /// The "strokedCheckmark" asset catalog image.
    static var strokedCheckmark: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .strokedCheckmark)
#else
        .init()
#endif
    }

    /// The "system" asset catalog image.
    static var system: AppKit.NSImage {
#if !targetEnvironment(macCatalyst)
        .init(resource: .system)
#else
        .init()
#endif
    }

}
#endif

#if canImport(UIKit)
@available(iOS 17.0, tvOS 17.0, *)
@available(watchOS, unavailable)
extension UIKit.UIImage {

    /// The "1stImage" asset catalog image.
    static var _1St: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: ._1St)
#else
        .init()
#endif
    }

    /// The "BigImageView" asset catalog image.
    static var bigImageView: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .bigImageView)
#else
        .init()
#endif
    }

    /// The "Icons" asset catalog resource namespace.
    enum Icons {

        /// The "Icons/Star" asset catalog image.
        static var star: UIKit.UIImage {
#if !os(watchOS)
            .init(resource: .Icons.star)
#else
            .init()
#endif
        }

    }

    /// The "Image" asset catalog image.
    static var image: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .image)
#else
        .init()
#endif
    }

    /// The "Image Name" asset catalog image.
    static var imageName: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .imageName)
#else
        .init()
#endif
    }

    /// The "Image2" asset catalog image.
    static var image2: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .image2)
#else
        .init()
#endif
    }

    /// The "Logo" asset catalog image.
    static var logo: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .logo)
#else
        .init()
#endif
    }

    /// The "Nested" asset catalog resource namespace.
    enum Nested {

        /// The "Nested/Deep" asset catalog resource namespace.
        enum Deep {

            /// The "Nested/Deep/Leaf" asset catalog image.
            static var leaf: UIKit.UIImage {
#if !os(watchOS)
                .init(resource: .Nested.Deep.leaf)
#else
                .init()
#endif
            }

        }

    }

    /// The "PadOnly" asset catalog image.
    static var padOnly: UIKit.UIImage? {
#if !os(watchOS)
        .init(thinnableResource: .padOnly)
#else
        .init()
#endif
    }

    #warning("The \"actions\" image asset name resolves to a conflicting UIImage symbol \"actions\". Try renaming the asset.")

    #warning("The \"add\" image asset name resolves to a conflicting UIImage symbol \"add\". Try renaming the asset.")

    /// The "branch" asset catalog image.
    static var branch: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .branch)
#else
        .init()
#endif
    }

    #warning("The \"checkmark\" image asset name resolves to a conflicting UIImage symbol \"checkmark\". Try renaming the asset.")

    /// The "highlightedBranch" asset catalog image.
    static var highlightedBranch: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .highlightedBranch)
#else
        .init()
#endif
    }

    /// The "init" asset catalog image.
    static var `init`: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .`init`)
#else
        .init()
#endif
    }

    /// The "logo-dark" asset catalog image.
    static var logoDark: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .logoDark)
#else
        .init()
#endif
    }

    /// The "my icons" asset catalog resource namespace.
    enum MyIcons {

        /// The "my icons/Home" asset catalog image.
        static var home: UIKit.UIImage {
#if !os(watchOS)
            .init(resource: .MyIcons.home)
#else
            .init()
#endif
        }

    }

    /// The "photo" asset catalog image.
    static var photo: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .photo)
#else
        .init()
#endif
    }

    #warning("The \"remove\" image asset name resolves to a conflicting UIImage symbol \"remove\". Try renaming the asset.")

    /// The "self" asset catalog image.
    static var `self`: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .`self`)
#else
        .init()
#endif
    }

    #warning("The \"strokedCheckmark\" image asset name resolves to a conflicting UIImage symbol \"strokedCheckmark\". Try renaming the asset.")

    /// The "system" asset catalog image.
    static var system: UIKit.UIImage {
#if !os(watchOS)
        .init(resource: .system)
#else
        .init()
#endif
    }

}
#endif

// MARK: - Thinnable Asset Support -

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
@available(watchOS, unavailable)
extension DeveloperToolsSupport.ColorResource {

    private init?(thinnableName: Swift.String, bundle: Foundation.Bundle) {
#if canImport(AppKit) && os(macOS)
        if AppKit.NSColor(named: NSColor.Name(thinnableName), bundle: bundle) != nil {
            self.init(name: thinnableName, bundle: bundle)
        } else {
            return nil
        }
#elseif canImport(UIKit) && !os(watchOS)
        if UIKit.UIColor(named: thinnableName, in: bundle, compatibleWith: nil) != nil {
            self.init(name: thinnableName, bundle: bundle)
        } else {
            return nil
        }
#else
        return nil
#endif
    }

}

#if canImport(AppKit)
@available(macOS 14.0, *)
@available(macCatalyst, unavailable)
extension AppKit.NSColor {

    private convenience init?(thinnableResource: DeveloperToolsSupport.ColorResource?) {
#if !targetEnvironment(macCatalyst)
        if let resource = thinnableResource {
            self.init(resource: resource)
        } else {
            return nil
        }
#else
        return nil
#endif
    }

}
#endif

#if canImport(UIKit)
@available(iOS 17.0, tvOS 17.0, *)
@available(watchOS, unavailable)
extension UIKit.UIColor {

    private convenience init?(thinnableResource: DeveloperToolsSupport.ColorResource?) {
#if !os(watchOS)
        if let resource = thinnableResource {
            self.init(resource: resource)
        } else {
            return nil
        }
#else
        return nil
#endif
    }

}
#endif

#if canImport(SwiftUI)
@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
extension SwiftUI.Color {

    private init?(thinnableResource: DeveloperToolsSupport.ColorResource?) {
        if let resource = thinnableResource {
            self.init(resource)
        } else {
            return nil
        }
    }

}

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
extension SwiftUI.ShapeStyle where Self == SwiftUI.Color {

    private init?(thinnableResource: DeveloperToolsSupport.ColorResource?) {
        if let resource = thinnableResource {
            self.init(resource)
        } else {
            return nil
        }
    }

}
#endif

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
@available(watchOS, unavailable)
extension DeveloperToolsSupport.ImageResource {

    private init?(thinnableName: Swift.String, bundle: Foundation.Bundle) {
#if canImport(AppKit) && os(macOS)
        if bundle.image(forResource: NSImage.Name(thinnableName)) != nil {
            self.init(name: thinnableName, bundle: bundle)
        } else {
            return nil
        }
#elseif canImport(UIKit) && !os(watchOS)
        if UIKit.UIImage(named: thinnableName, in: bundle, compatibleWith: nil) != nil {
            self.init(name: thinnableName, bundle: bundle)
        } else {
            return nil
        }
#else
        return nil
#endif
    }

}

#if canImport(AppKit)
@available(macOS 14.0, *)
@available(macCatalyst, unavailable)
extension AppKit.NSImage {

    private convenience init?(thinnableResource: DeveloperToolsSupport.ImageResource?) {
#if !targetEnvironment(macCatalyst)
        if let resource = thinnableResource {
            self.init(resource: resource)
        } else {
            return nil
        }
#else
        return nil
#endif
    }

}
#endif

#if canImport(UIKit)
@available(iOS 17.0, tvOS 17.0, *)
@available(watchOS, unavailable)
extension UIKit.UIImage {

    private convenience init?(thinnableResource: DeveloperToolsSupport.ImageResource?) {
#if !os(watchOS)
        if let resource = thinnableResource {
            self.init(resource: resource)
        } else {
            return nil
        }
#else
        return nil
#endif
    }

}
#endif

