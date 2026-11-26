import UIKit

@objc(MWMNavigationDashboardManager) @objcMembers
private final class DashboardOwner: NSObject {
    var navigationControlView: NavigationControl?
    func settingsButtonAction() {}
    func ttsButtonAction() {}
    func stopRoutingButtonAction() {}
}

@objc(MWMNavigationControlView) @objcMembers
private final class NavigationControl: UIView {
    var speedBackground, extendedView, activeTrackRecordingButton, mapPositionButton, progressView: UIView?
    var speedLabel, speedLegendLabel, speedWithLegendLabel, timeLabel: UILabel?
    var distanceLabel, distanceLegendLabel, distanceWithLegendLabel: UILabel?
    var trackRecordingButton, ttsButton, extendButton: UIButton?
    var timePageControl: UIPageControl?
    var routingProgress: NSLayoutConstraint?
    func toggleInfoAction() {}
    func extendAction() {}
    func trackRecordingAction() {}
}

@objc(MWMButton) private class MapButton: UIButton {}
@objc(MWMStopButton) private final class StopButton: MapButton {
    @objc var localizedText: String?
}

@available(iOS 17.0, *)
@MainActor func checkNavigationControl(_ nib: String, on parent: UIViewController) {
    let owner = DashboardOwner()
    let objects = UINib(nibName: nib, bundle: .main).instantiate(withOwner: owner)
    guard let view = objects.first as? NavigationControl, owner.navigationControlView === view,
          let speed = view.speedBackground,
          let width = speed.constraints.first(where: { $0.firstAttribute == .width && $0.secondItem == nil }) else {
        fatalError("Missing CoMaps speed panel or width constraint in \(nib)")
    }
    precondition(width.priority == .defaultHigh && width.isActive)
    precondition(view.extendButton?.image(for: .normal) != nil, "Missing CoMaps menu icon in \(nib)")
    let host = UIViewController()
    parent.addChild(host)
    host.view.frame = CGRect(x: 0, y: 0, width: 768, height: 200)
    parent.view.addSubview(host.view)
    host.didMove(toParent: parent)
    defer {
        host.willMove(toParent: nil)
        host.view.removeFromSuperview()
        host.removeFromParent()
    }
    host.view.addSubview(view)
    NSLayoutConstraint.activate([
        view.leadingAnchor.constraint(equalTo: host.view.leadingAnchor),
        view.trailingAnchor.constraint(equalTo: host.view.trailingAnchor),
        view.topAnchor.constraint(equalTo: host.view.topAnchor),
    ])
    func check(_ horizontal: UIUserInterfaceSizeClass, _ vertical: UIUserInterfaceSizeClass, _ points: CGFloat) {
        host.traitOverrides.horizontalSizeClass = horizontal
        host.traitOverrides.verticalSizeClass = vertical
        speed.updateTraitsIfNeeded()
        host.view.layoutIfNeeded()
        precondition(speed.traitCollection.horizontalSizeClass == horizontal && speed.traitCollection.verticalSizeClass == vertical)
        precondition(width.constant == points && speed.bounds.width == points,
                     "\(nib) \(horizontal.rawValue)/\(vertical.rawValue): expected \(points), got constant \(width.constant), width \(speed.bounds.width)")
    }
    check(.regular, .compact, 128)
    check(.compact, .compact, 72)
    check(.compact, .regular, 88)
    check(.regular, .regular, 88)
    check(.regular, .compact, 128)
}
