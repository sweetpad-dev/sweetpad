import UIKit

private func matchesRGB(_ color: UIColor?, _ red: CGFloat, _ green: CGFloat, _ blue: CGFloat) -> Bool {
    var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
    return color?.getRed(&r, green: &g, blue: &b, alpha: &a) == true &&
        abs(r - red) < 0.001 && abs(g - green) < 0.001 && abs(b - blue) < 0.001 && a == 1
}

@MainActor private func checkLabels(in bundle: Bundle) {
    let objects = UINib(nibName: "AttributedLabels", bundle: bundle).instantiate(withOwner: nil)
    guard let root = objects.first as? UIView else { fatalError("Missing label container in \(bundle)") }
    let labels = root.subviews.compactMap { $0 as? UILabel }
    precondition(labels.count == 4, "Missing labels in \(bundle)")
    let single = labels[0]
    guard let singleText = single.attributedText,
          let bold = singleText.attribute(.font, at: 0, effectiveRange: nil) as? UIFont else {
        fatalError("Missing single-run font in \(bundle)")
    }
    precondition(single.text == "Map details" && singleText.string == single.text)
    precondition(bold.fontName == "HelveticaNeue-Bold" && bold.pointSize == 15 && single.font == bold)
    guard let text = labels[1].attributedText else { fatalError("Missing mixed runs in \(bundle)") }
    precondition(text.string == "Go🌍 now" && text.length == 8)
    for (index, name, size) in [(0, "HelveticaNeue", 14), (4, "HelveticaNeue-Bold", 19)] {
        var range = NSRange()
        let font = text.attribute(.font, at: index, effectiveRange: &range) as? UIFont
        precondition(font?.fontName == name && font?.pointSize == CGFloat(size))
        precondition(range == NSRange(location: index, length: 4), "Wrong UTF-16 run in \(bundle)")
    }
    guard let red = text.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? UIColor else {
        fatalError("Missing foreground color in \(bundle)")
    }
    precondition(matchesRGB(red, 1, 0, 0))
    precondition(labels[2].text == "Map legend" && labels[2].textAlignment == .center)
    precondition(labels[3].attributedText?.length == 0 && labels[3].text == "")
}

@MainActor func checkResourceVariants(in bundle: Bundle) {
    checkLabels(in: bundle)
    let objects = UINib(nibName: "ButtonStateColors", bundle: bundle).instantiate(withOwner: nil)
    guard let button = objects.first as? UIButton else { fatalError("Missing button in \(bundle)") }
    precondition(matchesRGB(button.backgroundColor, 0, 1, 0), "Wrong button background in \(bundle)")
    precondition(matchesRGB(button.titleColor(for: .highlighted), 1, 1, 0), "Wrong inherited title color in \(bundle)")
    precondition(matchesRGB(button.titleShadowColor(for: .normal), 1, 0, 0), "Wrong normal shadow in \(bundle)")
    precondition(matchesRGB(button.titleShadowColor(for: .highlighted), 0, 0, 1), "Wrong highlighted shadow in \(bundle)")
    func check(_ name: String, _ width: UIUserInterfaceSizeClass, _ height: UIUserInterfaceSizeClass,
               _ rgb: [UInt8], style: UIUserInterfaceStyle = .light,
               idiom: UIUserInterfaceIdiom = .phone, scale: CGFloat = 1) {
        let traits: UITraitCollection
        if #available(iOS 17.0, *) {
            traits = UITraitCollection {
                $0.horizontalSizeClass = width
                $0.verticalSizeClass = height
                $0.userInterfaceStyle = style
                $0.userInterfaceIdiom = idiom
                $0.displayScale = scale
            }
        } else {
            traits = UITraitCollection(traitsFrom: [
                .init(horizontalSizeClass: width), .init(verticalSizeClass: height),
                .init(userInterfaceStyle: style), .init(userInterfaceIdiom: idiom), .init(displayScale: scale),
            ])
        }
        guard let image = UIImage(named: name, in: bundle, compatibleWith: traits),
              let raster = image.cgImage,
              let context = CGContext(data: nil, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                                      space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            fatalError("Could not read \(name) in \(bundle)")
        }
        context.draw(raster, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        let pixel = Array(UnsafeBufferPointer(start: context.data!.assumingMemoryBound(to: UInt8.self), count: 4))
        precondition(pixel == rgb + [255], "\(name) in \(bundle.bundleURL.lastPathComponent) for \(traits): \(pixel)")
    }
    check("MapPin", .regular, .regular, [255, 0, 0])
    check("MapPin", .compact, .regular, [0, 0, 255])
    check("ScaledMarker", .regular, .regular, [0, 255, 0], scale: 2)
    check("ScaledMarker", .regular, .regular, [0, 0, 255], scale: 3)
    check("AdaptiveMarker", .regular, .compact, [0, 255, 0])
    check("AdaptiveMarker", .compact, .compact, [255, 0, 255])
    check("AdaptiveMarker", .regular, .regular, [255, 255, 0])
    check("AdaptiveMarker", .compact, .regular, [0, 255, 255], style: .dark)
    check("AdaptiveMarker", .regular, .regular, [255, 128, 0], idiom: .pad, scale: 2)
    for scale in [1, 2, 3] {
        check("AdaptiveVector", .regular, .regular, [255, 0, 0], scale: CGFloat(scale))
        check("AdaptiveVector", .compact, .regular, [0, 0, 255], scale: CGFloat(scale))
    }
}
