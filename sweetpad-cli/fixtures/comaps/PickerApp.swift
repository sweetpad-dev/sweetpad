import UIKit

// Supply the outlets, actions and style hook the real CoMaps XIB references.
@objc(MWMOpeningHoursTimeSelectorTableViewCell)
final class TimeSelectorCell: UITableViewCell {
    @IBOutlet var openTimePicker: UIDatePicker!
    @IBOutlet var closeTimePicker: UIDatePicker!
    var actions: [String] = []
    @objc func openValueChanged() { actions.append("open") }
    @objc func closeValueChanged() { actions.append("close") }
}

extension UIView {
    @objc var styleName: String {
        get { "" }
        set { }
    }
}

@main final class App: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession,
                     options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: nil, sessionRole: session.role)
        config.delegateClass = Scene.self
        return config
    }
}

final class Scene: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: scene)
        let controller = UIViewController()
        window.rootViewController = controller
        self.window = window
        window.makeKeyAndVisible()
        DispatchQueue.main.async {
            let objects = UINib(nibName: "TimeSelector", bundle: .main).instantiate(withOwner: nil)
            guard let cell = objects.first as? TimeSelectorCell,
                  let open = cell.openTimePicker, let close = cell.closeTimePicker else {
                fatalError("TimeSelector.nib did not connect both date-picker outlets")
            }
            precondition(open !== close && [open, close].allSatisfy {
                $0.datePickerMode == .time && $0.minuteInterval == 5 &&
                $0.preferredDatePickerStyle == .wheels && $0.locale?.identifier == "ru"
            }, "Date-picker settings differ from the XIB")
            open.sendActions(for: .valueChanged)
            close.sendActions(for: .valueChanged)
            precondition(cell.actions == ["open", "close"], "Date pickers called the wrong actions")
            checkCoMapsImages()
            guard #available(iOS 17.0, *) else { fatalError("Trait override checks need an iOS 17 or newer simulator") }
            func variants(_ name: String) -> Bundle {
                guard let url = Bundle.main.url(forResource: name, withExtension: "bundle"),
                      let bundle = Bundle(url: url) else { fatalError("Missing \(name) bundle") }
                return bundle
            }
            let xcode = variants("XcodeVariants"), linux = variants("LinuxVariants")
            for bundle in [xcode, linux] {
                checkResourceVariants(in: bundle)
            }
            for nib in ["XcodeNavigation", "LinuxNavigation"] { checkNavigationControl(nib, on: controller) }
            checkImageDirections(linux: linux, xcode: xcode, on: controller)
            checkPDFMasks(linuxMasks: variants("LinuxPDFMasks"), xcodeMasks: variants("XcodePDFMasks"),
                          xcodeCoMaps: variants("XcodeCoMaps"))
            checkCustomSymbols(xcode: variants("XcodeCoMaps"), linuxWidget: variants("LinuxWidget"),
                               xcodeWidget: variants("XcodeWidget"))
            print("CoMaps resources and variant bundles passed")
            fflush(stdout)
            exit(0)
        }
    }
}
