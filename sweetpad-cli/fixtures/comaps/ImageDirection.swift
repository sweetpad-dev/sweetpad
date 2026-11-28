import UIKit

@available(iOS 17.0, *)
@MainActor func checkImageDirections(linux: Bundle, xcode: Bundle, on controller: UIViewController) {
    let side = 64
    func mask(_ name: String, in bundle: Bundle, rtl: Bool, scale: CGFloat = 1) -> [UInt8] {
        controller.traitOverrides.layoutDirection = rtl ? .rightToLeft : .leftToRight
        let box = UIView(frame: CGRect(x: 0, y: 0, width: side, height: side))
        box.semanticContentAttribute = rtl ? .forceRightToLeft : .forceLeftToRight
        controller.view.addSubview(box)
        defer { box.removeFromSuperview() }
        let view = UIImageView(frame: box.bounds)
        view.semanticContentAttribute = box.semanticContentAttribute
        view.tintColor = .black
        box.addSubview(view)
        view.traitOverrides.displayScale = scale
        view.traitOverrides.userInterfaceStyle = .light
        view.updateTraitsIfNeeded()
        precondition(view.traitCollection.displayScale == scale,
                     "\(name) source scale \(scale), image view scale \(view.traitCollection.displayScale)")
        let traits = view.traitCollection.modifyingTraits {
            $0.layoutDirection = rtl ? .rightToLeft : .leftToRight
            $0.displayScale = scale
            $0.userInterfaceStyle = .light
        }
        guard let image = UIImage(named: name, in: bundle, compatibleWith: traits) else {
            fatalError("Missing \(name) in \(bundle.bundleURL.lastPathComponent)")
        }
        view.image = image
        box.layoutIfNeeded()
        precondition(view.effectiveUserInterfaceLayoutDirection == (rtl ? .rightToLeft : .leftToRight))
        let context = CGContext(data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4,
                                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.clear(box.bounds)
        box.layer.render(in: context)
        let pixels = context.data!.assumingMemoryBound(to: UInt8.self)
        let alpha = (0..<(side * side)).map { pixels[$0 * 4 + 3] }
        precondition(alpha.contains(0) && alpha.contains { $0 > 0 }, "\(name) rendered an empty or solid mask")
        return alpha
    }
    func mirror(_ alpha: [UInt8]) -> [UInt8] {
        alpha.indices.map { alpha[($0 / side) * side + side - 1 - $0 % side] }
    }
    func checkMirror(_ name: String, in bundle: Bundle, scale: CGFloat) {
        let ltr = mask(name, in: bundle, rtl: false, scale: scale)
        let rtl = mask(name, in: bundle, rtl: true, scale: scale)
        precondition(ltr != rtl && mirror(ltr) == rtl, "\(name) failed to mirror at \(scale)x in \(bundle.bundleURL.lastPathComponent)")
    }
    for name in ["ic_24px_manager", "ic_menu_question", "ic_search_mode_list"] {
        for scale: CGFloat in [1, 2, 3] { checkMirror(name, in: .main, scale: scale) }
    }
    let explicitRight = mask("Pair", in: xcode, rtl: true)
    for bundle in [xcode, linux] {
        let fixed = mask("Neutral", in: bundle, rtl: false)
        precondition(fixed != mirror(fixed) && mask("Neutral", in: bundle, rtl: true) == fixed)
        for name in ["Forward", "Vector", "Preserved"] {
            for scale: CGFloat in [1, 2, 3] { checkMirror(name, in: bundle, scale: scale) }
        }
        checkMirror("Reverse", in: bundle, scale: 1)
        precondition(mask("Forward", in: bundle, rtl: false) == fixed, "Wrong LTR source orientation in \(bundle)")
        precondition(mask("Reverse", in: bundle, rtl: true) == fixed, "Wrong RTL source orientation in \(bundle)")
        precondition(mask("Pair", in: bundle, rtl: false) == fixed, "Wrong explicit LTR artwork in \(bundle)")
        precondition(explicitRight != fixed && explicitRight != mirror(fixed))
        precondition(mask("Pair", in: bundle, rtl: true) == explicitRight, "Wrong explicit RTL artwork in \(bundle)")
    }
}
