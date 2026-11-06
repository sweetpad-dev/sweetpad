import Foundation
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

// MARK: - Thinnable Asset Support -

@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)
@available(watchOS, unavailable)
extension DeveloperToolsSupport.ColorResource {

    private init?(thinnableName: Swift.String, bundle: Foundation.Bundle) {
        return nil
    }

}

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
        return nil
    }

}

