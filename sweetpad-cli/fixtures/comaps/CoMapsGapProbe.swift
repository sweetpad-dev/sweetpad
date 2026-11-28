import CryptoKit
import UIKit
import WebKit

@objc(CoMapsProbeLayout) private final class CoMapsProbeLayout: UICollectionViewLayout {}

@MainActor private func rgba(_ color: UIColor?) -> String {
    guard let color else { return "nil" }
    var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
    guard color.getRed(&r, green: &g, blue: &b, alpha: &a) else { return color.description }
    return [r, g, b, a].map { String(Int(($0 * 255).rounded())) }.joined(separator: ",")
}

@MainActor private func pixels(_ image: UIImage?, scale: CGFloat = 1) -> String {
    guard let image else { return "nil" }
    let format = UIGraphicsImageRendererFormat()
    format.scale = scale
    format.preferredRange = .standard
    let size = image.size
    let rendered = UIGraphicsImageRenderer(size: size, format: format).image { _ in
        image.withRenderingMode(.alwaysOriginal).draw(in: CGRect(origin: .zero, size: size))
    }
    let digest = SHA256.hash(data: rendered.pngData()!).map { String(format: "%02x", $0) }.joined()
    return "\(size.width)x\(size.height)@\(image.scale) \(digest)"
}

@available(iOS 17.0, *)
@MainActor private func gapObservations(_ bundle: Bundle, on parent: UIViewController) -> [String: String] {
    var values: [String: String] = [:]
    func nib(_ name: String) -> [Any] { UINib(nibName: name, bundle: bundle).instantiate(withOwner: nil) }
    func view<T: UIView>(_ root: UIView, _ tag: Int, _: T.Type) -> T {
        guard let result = root.viewWithTag(tag) as? T else { fatalError("Missing \(T.self) with tag \(tag) in \(bundle)") }
        return result
    }
    let controls = nib("CoMapsControls")[0] as! UIView
    let button = view(controls, 101, UIButton.self)
    let label = view(controls, 102, UILabel.self)
    let field = view(controls, 103, UITextField.self)
    let text = view(controls, 104, UITextView.self)
    let plain = view(controls, 105, UITextView.self)
    let pages = view(controls, 106, UIPageControl.self)
    values["button.accessibility"] = button.accessibilityIdentifier
    values["button.autoresizing"] = String(button.autoresizingMask.rawValue)
    values["button.contentInsets"] = NSCoder.string(for: button.contentEdgeInsets)
    values["button.titleInsets"] = NSCoder.string(for: button.titleEdgeInsets)
    values["button.normalImage"] = pixels(button.image(for: .normal))
    values["button.selectedImage"] = pixels(button.image(for: .selected))
    let selectedConfiguration = button.preferredSymbolConfigurationForImage(in: .selected)
    precondition(selectedConfiguration != nil, "Missing selected button symbol configuration in \(bundle)")
    values["button.configuredImage"] = pixels(button.image(for: .selected)?.applyingSymbolConfiguration(selectedConfiguration!))
    values["button.selectedTitle"] = button.title(for: .selected)
    values["label.traits"] = String(label.accessibilityTraits.rawValue)
    values["label.shadowOffset"] = NSCoder.string(for: label.shadowOffset)
    values["field.background"] = rgba(field.backgroundColor)
    values["field.margins"] = NSCoder.string(for: field.layoutMargins)
    values["text.accessibility"] = text.accessibilityIdentifier
    values["text.background"] = rgba(text.backgroundColor)
    values["text.richText"] = text.attributedText.string
    let rich = text.attributedText.attributes(at: 0, effectiveRange: nil)
    values["text.font"] = (rich[.font] as? UIFont)?.fontName
    values["text.pointSize"] = (rich[.font] as? UIFont).map { String(describing: $0.pointSize) }
    values["text.foreground"] = rgba(rich[.foregroundColor] as? UIColor)
    values["text.viewFont"] = text.font?.fontName
    values["text.viewColor"] = rgba(text.textColor)
    values["text.plainText"] = plain.text
    values["page.background"] = rgba(pages.backgroundColor)
    values["view.runtimeReal"] = String(describing: controls.layer.cornerRadius)

    let lists = nib("CoMapsListViews")[0] as! UIView
    let table = view(lists, 201, UITableView.self)
    let collection = view(lists, 202, UICollectionView.self)
    let search = view(lists, 203, UISearchBar.self)
    let stack = view(lists, 204, UIStackView.self)
    let web = view(lists, 205, WKWebView.self)
    values["table.separator"] = rgba(table.separatorColor)
    values["table.sectionIndex"] = rgba(table.sectionIndexBackgroundColor)
    values["table.tracking"] = rgba(table.sectionIndexTrackingBackgroundColor)
    values["table.separatorInset"] = NSCoder.string(for: table.separatorInset)
    values["table.indicatorInsets"] = NSCoder.string(for: table.verticalScrollIndicatorInsets)
    values["collection.layout"] = NSStringFromClass(type(of: collection.collectionViewLayout))
    values["search.background"] = rgba(search.backgroundColor)
    values["search.barTint"] = rgba(search.barTintColor)
    values["search.input"] = String(search.autocapitalizationType.rawValue)
    values["search.defaultHeight"] = String(search.constraints.contains { $0.firstAttribute == .height && $0.constant == 55 })
    values["stack.background"] = rgba(stack.backgroundColor)
    values["stack.margins"] = NSCoder.string(for: stack.layoutMargins)
    values["stack.relativeMargins"] = String(stack.isLayoutMarginsRelativeArrangement)
    values["web.background"] = rgba(web.backgroundColor)
    values["web.media"] = String(web.configuration.mediaTypesRequiringUserActionForPlayback.rawValue)

    let cells = nib("CoMapsCells")
    let cell = cells.compactMap { $0 as? UITableViewCell }.first!
    let item = cells.compactMap { $0 as? UICollectionViewCell }.first!
    let picker = view(cell, 302, UIDatePicker.self)
    values["cell.tint"] = rgba(cell.tintColor)
    values["cell.contentTint"] = rgba(cell.contentView.tintColor)
    values["cell.contentBackground"] = rgba(cell.contentView.backgroundColor)
    for (name, owner) in [("cell", cell as UIView), ("item", item.contentView)] {
        let constraint = owner.constraints.first { $0.identifier == "\(name)-safe-width" }
        let connected = owner.safeAreaLayoutGuide.owningView === owner && constraint?.firstItem === owner.safeAreaLayoutGuide
        precondition(connected, "Disconnected \(name) safe area in \(bundle)")
        values[name + ".safeArea"] = String(connected)
    }
    values["item.frame"] = NSCoder.string(for: item.frame)
    values["item.contentFrame"] = NSCoder.string(for: item.contentView.frame)
    values["picker.locale"] = picker.locale?.identifier
    values["picker.frame"] = NSCoder.string(for: picker.frame)
    values["picker.mode"] = String(picker.datePickerMode.rawValue)

    let navigation = UIStoryboard(name: "CoMapsNavigation", bundle: bundle).instantiateInitialViewController() as! UINavigationController
    let container = view(navigation.viewControllers[0].view, 501, UIView.self)
    values["container.background"] = rgba(container.backgroundColor)
    values["container.embedded"] = String(navigation.viewControllers[0].children.contains { $0.view.superview === container })
    values["navigation.preferredSize"] = NSCoder.string(for: navigation.preferredContentSize)

    let adaptive = nib("CoMapsAdaptiveViews")[0] as! UIView
    values["adaptive.bounds"] = NSCoder.string(for: adaptive.bounds)
    let modern = nib("CoMapsAdaptiveModern")[0] as! UIView
    values["adaptive.modernBounds"] = NSCoder.string(for: modern.bounds)
    precondition(adaptive.bounds.size == CGSize(width: 414, height: 896) && modern.bounds.size == CGSize(width: 320, height: 480),
                 "Root view sizes for iOS 15.6 and iOS 17 differ in \(bundle)")
    let arrow = view(adaptive, 401, UIImageView.self)
    values["image.background"] = rgba(arrow.backgroundColor)
    let host = UIViewController()
    parent.addChild(host)
    host.view.frame = CGRect(x: 0, y: 0, width: 320, height: 480)
    parent.view.addSubview(host.view)
    host.didMove(toParent: parent)
    host.view.addSubview(adaptive)
    defer {
        host.willMove(toParent: nil)
        host.view.removeFromSuperview()
        host.removeFromParent()
    }
    for width: UIUserInterfaceSizeClass in [.compact, .regular] {
        for height: UIUserInterfaceSizeClass in [.regular, .compact] {
            host.traitOverrides.horizontalSizeClass = width
            host.traitOverrides.verticalSizeClass = height
            arrow.updateTraitsIfNeeded()
            host.view.layoutIfNeeded()
            let key = "adaptive.\(width.rawValue).\(height.rawValue)"
            values[key + ".frame"] = NSCoder.string(for: arrow.frame)
            values[key + ".image"] = pixels(arrow.image)
            let name = width == .regular && height == .regular ? "MapIndicator" : "RouteMarker"
            let named = UIImage(named: name, in: bundle, compatibleWith: arrow.traitCollection)
            precondition(named != nil && arrow.image?.size == named?.size && arrow.image!.size.width > 1,
                         "Wrong adaptive image for \(key) in \(bundle): expected \(name), got \(String(describing: arrow.image))")
            values[key + ".active"] = adaptive.constraints.filter { $0.isActive }.map {
                "\($0.firstAttribute.rawValue):\($0.constant):\($0.priority.rawValue)"
            }.sorted().joined(separator: ",")
        }
    }
    for style: UIUserInterfaceStyle in [.light, .dark] {
        for scale: CGFloat in [1, 2, 3] {
            let traits = UITraitCollection(traitsFrom: [.init(userInterfaceStyle: style), .init(displayScale: scale)])
            traits.performAsCurrent {
                let key = "asset.\(style.rawValue).\(scale)"
                values[key + ".color"] = rgba(UIColor(named: "MapAccent", in: bundle, compatibleWith: traits)?.resolvedColor(with: traits))
                for name in ["MapIndicator", "RouteMarker"] {
                    values[key + "." + name] = pixels(UIImage(named: name, in: bundle, compatibleWith: traits), scale: scale)
                }
            }
        }
    }
    return values
}

@available(iOS 17.0, *)
@MainActor func checkCoMapsGaps(linux: Bundle, xcode: Bundle, on parent: UIViewController) {
    let expected = gapObservations(xcode, on: parent)
    let actual = gapObservations(linux, on: parent)
    let differences = Set(expected.keys).union(actual.keys).sorted().compactMap { key -> String? in
        guard expected[key] != actual[key] else { return nil }
        return "\(key): Xcode \(expected[key] ?? "<missing>"), Linux \(actual[key] ?? "<missing>")"
    }
    precondition(differences.isEmpty, differences.joined(separator: "\n"))
    precondition(expected["text.plainText"] == "First line\nSecond line" && expected["web.media"] == "0")
    precondition(expected["search.barTint"] == "255,0,0,255" && expected["stack.relativeMargins"] == "true")
    precondition(expected["table.indicatorInsets"] == "{44, 0, 0, 0}" && expected["picker.locale"] == "ru")
    precondition(expected["search.defaultHeight"] == "false" && expected["container.embedded"] == "true")
    precondition(expected["button.normalImage"] != "nil" && expected["button.configuredImage"] != expected["button.selectedImage"])
    print("Compared CoMaps interface and asset fixtures: \(expected.count) UIKit observations matched Xcode")
}
