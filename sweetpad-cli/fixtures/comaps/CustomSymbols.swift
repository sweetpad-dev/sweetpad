import UIKit

private struct SymbolPixels {
    let width: Int
    let height: Int
    let rgba: [UInt8]

    @MainActor init(_ image: UIImage, scale: CGFloat) {
        let bounds = CGRect(x: 0, y: 0, width: ceil(image.size.width) + 4, height: ceil(image.size.height) + 4)
        let view = UIImageView(frame: bounds)
        view.contentMode = .center
        view.tintColor = UIColor(red: 0.85, green: 0.09, blue: 0.67, alpha: 1)
        view.image = image
        width = Int(bounds.width * scale)
        height = Int(bounds.height * scale)
        let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.scaleBy(x: scale, y: scale)
        view.layer.render(in: context)
        rgba = Array(UnsafeBufferPointer(start: context.data!.assumingMemoryBound(to: UInt8.self), count: width * height * 4))
    }

    func compare(with reference: Self, cached: Bool, fractional: Bool, label: String) {
        precondition(width == reference.width && height == reference.height, "Raster dimensions differ for \(label)")
        precondition(stride(from: 3, to: reference.rgba.count, by: 4).contains { reference.rgba[$0] != 0 },
                     "Empty Xcode symbol for \(label)")
        if rgba == reference.rgba { return }
        let offsets = [-width - 1, -width, -width + 1, -1, 0, 1, width - 1, width, width + 1]
        var ink = 0, difference = 0
        for y in 0..<height {
            for x in 0..<width {
                let i = (y * width + x) * 4
                ink += Int(reference.rgba[i + 3])
                var worst = 0
                for channel in 0..<4 {
                    let j = i + channel
                    let value = Int(rgba[j]), expected = Int(reference.rgba[j])
                    let delta = abs(value - expected)
                    worst = max(worst, delta)
                    if delta <= 2 { continue }
                    // Xcode can resample a cached bitmap near 15, 17 and 20 pt.
                    // A contour pixel may blend adjacent colors. Flat interiors must agree.
                    precondition(cached && x > 0 && y > 0 && x + 1 < width && y + 1 < height,
                                 "\(label) at (\(x), \(y)) channel \(channel): expected \(expected), got \(value)")
                    precondition(offsets.contains { reference.rgba[j + $0 * 4] != reference.rgba[j] },
                                 "\(label) at (\(x), \(y)) channel \(channel): expected flat \(expected), got \(value)")
                }
                difference += worst
            }
        }
        // A fractional point size also resizes the cached raster. Bound the total
        // error against visible ink so a missing stroke cannot pass in a large frame.
        precondition(difference * (fractional ? 8 : 50) <= ink,
                     "\(label) differs by \(difference) channel levels over \(ink) alpha levels")
    }
}

@available(iOS 17.0, *)
@MainActor func checkCustomSymbols(xcode: Bundle, linuxWidget: Bundle, xcodeWidget: Bundle) {
    let url = Bundle.main.url(forResource: "CoMapsSymbols", withExtension: "json")!
    let names = try! JSONDecoder().decode([String: [String]].self, from: Data(contentsOf: url))
    let sizes: [(CGFloat, UIImage.SymbolWeight, UIImage.SymbolScale, CGFloat, UIUserInterfaceStyle)] = [
        (15, .regular, .medium, 2, .light), (17, .regular, .medium, 3, .light), (20, .regular, .medium, 1, .dark),
        (17.03, .regular, .medium, 3, .light), (29, .bold, .large, 2, .dark), (21, .ultraLight, .small, 1, .light)
    ]
    let modes: [(String, UIImage.SymbolConfiguration?, Double?)] = [
        ("default", nil, nil), ("monochrome", .preferringMonochrome(), nil),
        ("multicolor", .preferringMulticolor(), nil), ("hierarchical", .init(hierarchicalColor: .systemOrange), nil),
        ("palette", .init(paletteColors: [.systemIndigo, .systemYellow, .systemTeal]), nil),
        ("variable 0", .preferringMulticolor(), 0), ("variable 0.75", .preferringMulticolor(), 0.75)
    ]
    var comparisons = 0
    for (catalog, linux, native) in [("app", Bundle.main, xcode), ("widget", linuxWidget, xcodeWidget)] {
        for (points, weight, size, display, style) in sizes {
            let traits = UITraitCollection {
                $0.displayScale = display
                $0.userInterfaceStyle = style
                $0.layoutDirection = .leftToRight
            }
            let base = UIImage.SymbolConfiguration(pointSize: points, weight: weight, scale: size)
            traits.performAsCurrent {
                for name in names[catalog]! {
                    for (mode, colors, variable) in modes {
                        autoreleasepool {
                            let config = (colors.map { base.applying($0) } ?? base).withTraitCollection(traits)
                            let label = "\(catalog)/\(name) \(mode), \(points)pt at \(display)x"
                            func load(_ bundle: Bundle) -> UIImage {
                                let image: UIImage?
                                if let variable {
                                    image = UIImage(named: name, in: bundle, variableValue: variable, configuration: config)
                                } else {
                                    image = UIImage(named: name, in: bundle, with: config)
                                }
                                guard let image, image.isSymbolImage else { fatalError("Missing \(label) in \(bundle)") }
                                return image
                            }
                            let test = load(linux), reference = load(native)
                            precondition(test.size == reference.size && test.baselineOffsetFromBottom == reference.baselineOffsetFromBottom
                                         && test.alignmentRectInsets == reference.alignmentRectInsets,
                                         "Symbol layout differs for \(label): \(test.size) vs \(reference.size)")
                            SymbolPixels(test, scale: display).compare(with: SymbolPixels(reference, scale: display),
                                cached: weight == .regular && size == .medium, fractional: points != points.rounded(), label: label)
                            comparisons += 1
                        }
                    }
                }
            }
        }
    }
    print("Compared CoMaps symbols in UIKit: \(names["app"]!.count) app and \(names["widget"]!.count) widget sets, \(comparisons) renderings")
}
