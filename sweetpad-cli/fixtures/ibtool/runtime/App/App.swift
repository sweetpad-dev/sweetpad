import UIKit
import WebKit
import MapKit

@main final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ app: UIApplication, configurationForConnecting session: UISceneSession, options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: nil, sessionRole: session.role)
        configuration.delegateClass = SceneDelegate.self
        return configuration
    }
}
final class AuditSegue: UIStoryboardSegue {
    override func perform() {
        source.addChild(destination)
        source.view.addSubview(destination.view)
        destination.didMove(toParent: source)
    }
}
final class AuditOwner: NSObject { @objc var views: [UIView] = [] }
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        setvbuf(stdout, nil, _IOLBF, 0)
        let window = UIWindow(windowScene: scene)
        window.rootViewController = UIViewController()
        self.window = window
        window.makeKeyAndVisible()
        DispatchQueue.main.async { Audit.run(window: window) }
    }
}
@MainActor enum Audit {
    static func check(_ condition: @autoclosure () -> Bool, _ name: String) {
        let passed = condition()
        print("IBAUDIT \(name)=\(passed)")
        if !passed { exit(1) }
    }
    static func pump(_ view: UIView) {
        view.setNeedsLayout()
        view.layoutIfNeeded()
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.02))
    }
    static func run(window: UIWindow) {
        let owner = AuditOwner()
        let mutationViews = UINib(nibName: "MutationProperties", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!.subviews
        let indicators = mutationViews.compactMap { $0 as? UIActivityIndicatorView }
        check(indicators.map { $0.contentHuggingPriority(for: .horizontal).rawValue } == [751, 750, 750], "indicator-horizontal-priorities")
        check(indicators.map { $0.contentHuggingPriority(for: .vertical).rawValue } == [750, 751, 750], "indicator-vertical-priorities")
        let mutationSwitch = mutationViews.compactMap { $0 as? UISwitch }.first!
        check(mutationSwitch.contentHuggingPriority(for: .horizontal).rawValue == 751 && mutationSwitch.contentHuggingPriority(for: .vertical).rawValue == 751, "switch-priorities")
        check(mutationViews.compactMap { $0 as? UILabel }.first!.minimumScaleFactor == 0, "zero-minimum-scale")
        check(mutationViews[5].isHidden, "runtime-string-boolean")
        let configurationButtons = UINib(nibName: "ButtonConfigurations", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!.subviews.compactMap { $0 as? UIButton }
        check(configurationButtons.count == 20 && configurationButtons.allSatisfy { $0.configuration?.title == "Press" }, "button-configuration-styles")
        let cornerStyles: [UIButton.Configuration.CornerStyle] = [.dynamic, .small, .medium, .large, .capsule]
        check(configurationButtons.enumerated().allSatisfy { index, button in button.configuration?.cornerStyle == cornerStyles[index % 5] }, "button-configuration-corners")
        let properties = UINib(nibName: "AuditProperties", bundle: nil).instantiate(withOwner: owner).compactMap { $0 as? UIView }.first!
        check(owner.views.count == 1 && owner.views.first === properties, "outlet-collection")
        check(properties.semanticContentAttribute == .forceRightToLeft, "semantic-direction")
        check(properties.accessibilityIdentifier == "audit-id", "accessibility-identifier")
        check(properties.accessibilityLabel == "Audit label", "accessibility-label")
        check(properties.accessibilityHint == "Audit hint", "accessibility-hint")
        check(properties.isAccessibilityElement && properties.accessibilityTraits.contains(.button), "accessibility-traits")
        for name in ["Variations", "VariationHidden", "VariationColor", "VariationMask", "VariationMaskDefault"] {
            let root = UINib(nibName: name, bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!
            window.rootViewController!.view.addSubview(root)
            root.traitOverrides.horizontalSizeClass = .regular
            root.traitOverrides.verticalSizeClass = .regular
            pump(root)
            let label = root.subviews.compactMap { $0 as? UILabel }.first
            if name == "Variations" { check(label?.font.pointSize == 17, "font-regular") }
            if name == "VariationMaskDefault" { check(root.subviews.isEmpty, "uninstalled-default") }
            root.traitOverrides.horizontalSizeClass = .compact
            pump(root)
            if name == "Variations" { check(label?.font.pointSize == 23, "font-compact") }
            if name == "VariationHidden" { check(label?.isHidden == true, "hidden-compact") }
            if name == "VariationColor" { check(label?.textColor == UIColor.red, "color-compact") }
            if name == "VariationMask" { check(root.subviews.isEmpty, "excluded-compact") }
            if name == "VariationMaskDefault" { check(root.subviews.count == 1, "installed-compact") }
            root.traitOverrides.horizontalSizeClass = .regular
            pump(root)
            if name == "Variations" { check(label?.font.pointSize == 17, "font-restored") }
            if name == "VariationHidden" { check(label?.isHidden == false, "hidden-restored") }
            if name == "VariationMask" { check(root.subviews.count == 1, "subview-restored") }
            if name == "VariationMaskDefault" { check(root.subviews.isEmpty, "subview-uninstalled") }
            root.removeFromSuperview()
        }
        for name in ["VariationConstraint", "VariationConstraintMask", "VariationMultiple"] {
            let root = UINib(nibName: name, bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!
            window.rootViewController!.view.addSubview(root)
            root.traitOverrides.horizontalSizeClass = .regular
            root.traitOverrides.verticalSizeClass = .regular
            pump(root)
            let labels = root.subviews.compactMap { $0 as? UILabel }
            let constraint = name == "VariationConstraint" ? labels.first!.constraints.first { $0.identifier == "audit-width" }! : name == "VariationConstraintMask" ? root.constraints.first { $0.identifier == "audit-width" }! : nil
            if name == "VariationConstraint" { check(constraint?.constant == 150 && constraint?.priority.rawValue == 750, "constraint-regular") }
            if name == "VariationConstraintMask" { check(constraint?.isActive == true, "constraint-installed") }
            if name == "VariationMultiple" { check(labels.count == 2 && labels.allSatisfy { $0.font.pointSize == 19 && !$0.isHidden }, "multiple-regular") }
            root.traitOverrides.horizontalSizeClass = .compact
            pump(root)
            if name == "VariationConstraint" { check(constraint?.constant == 123 && constraint?.priority.rawValue == 750, "constraint-compact") }
            if name == "VariationConstraintMask" { check(constraint?.isActive == false, "constraint-excluded") }
            if name == "VariationMultiple" { check(labels.allSatisfy { $0.font.pointSize == 23 && $0.isHidden }, "multiple-compact") }
            root.traitOverrides.horizontalSizeClass = .regular
            pump(root)
            if name == "VariationConstraint" { check(constraint?.constant == 150, "constraint-restored") }
            if name == "VariationConstraintMask" { check(constraint?.isActive == true, "constraint-reinstalled") }
            root.removeFromSuperview()
        }
        let tabs = UIStoryboard(name: "TabController", bundle: nil).instantiateInitialViewController() as! UITabBarController
        check(tabs.viewControllers?.count == 1 && tabs.tabBar.items?.first?.title == "Articles", "tab-controller")
        let tabViews = UINib(nibName: "TabItems", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!.subviews
        let tabBar = tabViews.compactMap { $0 as? UITabBar }.first!
        check(tabBar.items?.count == 2 && tabBar.items?.first?.tag == 7 && tabBar.items?.first?.badgeValue == "3", "tab-items")
        let embedded = UIStoryboard(name: "Embed", bundle: nil).instantiateInitialViewController()!
        _ = embedded.view
        check(embedded.children.count == 1 && embedded.children.first?.view.superview != nil, "embed-segue")
        let custom = UIStoryboard(name: "CustomSegue", bundle: nil).instantiateInitialViewController()!
        custom.performSegue(withIdentifier: "present", sender: nil)
        check(custom.children.count == 1, "custom-segue")
        let integerViews = UINib(nibName: "IntegerWidths", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!.subviews
        check(integerViews.map { $0.tag } == [127,128,255,256,32767,32768,65535,65536,2147483647,2147483648,4294967295,-1,-2147483648,-2147483649], "integer-widths")
        let originalRoot = window.rootViewController
        UIView.setAnimationsEnabled(false)
        for name in ["ButtonSegue", "BarButtonSegue"] {
            let source = UIStoryboard(name: name, bundle: nil).instantiateInitialViewController()!
            window.rootViewController = source
            pump(source.view)
            if name == "ButtonSegue" {
                source.view.subviews.compactMap { $0 as? UIButton }.first!.sendActions(for: .touchUpInside)
            } else {
                let item = source.navigationItem.rightBarButtonItem!
                check(item.action == NSSelectorFromString("perform:") && item.target != nil, "bar-segue-action")
                UIApplication.shared.sendAction(item.action!, to: item.target, from: item, for: nil)
            }
            RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.4))
            check(source.presentedViewController != nil, "trigger-\(name)")
            source.dismiss(animated: false)
        }
        window.rootViewController = originalRoot
        UIView.setAnimationsEnabled(true)
        let views = UINib(nibName: "AdditionalViewVariants", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!.subviews
        let progress = views.compactMap { $0 as? UIProgressView }.first!
        check(progress.progress == 0.375 && progress.progressViewStyle == .bar, "progress-view")
        let stepper = views.compactMap { $0 as? UIStepper }.first!
        check(stepper.value == 3 && stepper.minimumValue == 1 && stepper.maximumValue == 12 && stepper.stepValue == 0.5, "stepper-values")
        check(!stepper.isContinuous && !stepper.autorepeat && stepper.wraps, "stepper-flags")
        let page = views.compactMap { $0 as? UIPageControl }.first!
        check(page.numberOfPages == 5 && page.currentPage == 2 && page.hidesForSinglePage, "page-control")
        let segmented = views.compactMap { $0 as? UISegmentedControl }.first!
        check(segmented.numberOfSegments == 2 && segmented.titleForSegment(at: 0) == "First" && segmented.isMomentary, "segmented-control")
        let textView = views.compactMap { $0 as? UITextView }.first!
        check(textView.text == "Editable text" && textView.font?.pointSize == 17, "text-view")
        check(views.compactMap { $0 as? UISearchBar }.first?.text == "", "search-bar")
        check(views.compactMap { $0 as? UIToolbar }.first?.items?.isEmpty == true, "toolbar")
        check(views.compactMap { $0 as? UITabBar }.first?.items?.isEmpty == true, "tab-bar")
        let frameworkViews = UINib(nibName: "FrameworkViewVariants", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!.subviews
        let web = frameworkViews.compactMap { $0 as? WKWebView }.first!
        check(web.customUserAgent == "AuditAgent" && web.allowsBackForwardNavigationGestures && !web.allowsLinkPreview, "web-view")
        check(web.configuration.preferences.minimumFontSize == 14 && !web.configuration.preferences.javaScriptEnabled, "web-preferences")
        let map = frameworkViews.compactMap { $0 as? MKMapView }.first!
        check(map.mapType == .hybrid && map.showsTraffic && !map.showsCompass && map.showsScale, "map-view")
        let propertyView = UINib(nibName: "ViewProperties", bundle: nil).instantiate(withOwner: nil).compactMap { $0 as? UIView }.first!
        check(abs(propertyView.alpha - 0.45) < 0.0001 && propertyView.tag == 17 && propertyView.restorationIdentifier == "audit-view", "view-properties")
        let navigation = UIStoryboard(name: "ReferenceNavigation", bundle: nil).instantiateInitialViewController() as! UINavigationController
        check(navigation.topViewController?.title == "External target", "external-navigation-root")
        let reference = UIStoryboard(name: "References", bundle: nil).instantiateViewController(withIdentifier: "UIViewController-Ref-aa-Vc1")
        check(reference.title == "External target", "external-scene-reference")
        let localized = UIStoryboard(name: "Localization", bundle: nil).instantiateInitialViewController()!
        let text = localized.view.subviews.compactMap { $0 as? UILabel }.first?.text
        let expected = ProcessInfo.processInfo.arguments.contains("--expect-french") ? "Bonjour" : "Hello"
        check(text == expected, "localization-\(expected)")
        print("IBAUDIT done")
        exit(0)
    }
}
