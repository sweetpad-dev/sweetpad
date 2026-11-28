import UIKit

private struct SymbolMaster: Hashable, Decodable {
    let name, weight, size: String
}

private struct CachedMaster: Hashable, Decodable {
    let name, weight, size: String
    let display: Int
}

private struct SymbolCatalog: Decodable {
    let names: [String]
    let generated: Set<SymbolMaster>
    let cached: Set<CachedMaster>
}

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

    func difference(from reference: Self, cached: Bool, fractional: Bool, generated: Bool) -> String? {
        guard width == reference.width && height == reference.height else { return "raster dimensions differ" }
        guard stride(from: 3, to: reference.rgba.count, by: 4).contains(where: { reference.rgba[$0] != 0 }) else {
            return "empty Xcode symbol"
        }
        if rgba == reference.rgba { return nil }
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
                    // Xcode 26 also bakes an intermediate vector that Xcode 27 leaves to UIKit.
                    // Comparing the two Apple catalogs gives a maximum of 21 levels at its contours.
                    guard (cached || (generated && delta <= 21)),
                          x > 0 && y > 0 && x + 1 < width && y + 1 < height,
                          offsets.contains(where: { reference.rgba[j + $0 * 4] != reference.rgba[j] }) else {
                        return "pixel (\(x), \(y)) channel \(channel): expected \(expected), got \(value)"
                    }
                }
                difference += worst
            }
        }
        // A fractional point size also resizes the cached raster. Bound the total
        // error against visible ink so a missing stroke cannot pass in a large frame.
        // The largest difference between Apple's baked and interpolated weights was 2.499% of ink.
        let divisor = cached && fractional ? 8 : (generated ? 40 : 50)
        guard difference * divisor <= ink else {
            return "difference of \(difference) channel levels over \(ink) alpha levels"
        }
        return nil
    }
}

@available(iOS 17.0, *)
@MainActor func checkCustomSymbols(xcode: Bundle, linuxWidget: Bundle, xcodeWidget: Bundle) {
    let url = Bundle.main.url(forResource: "CoMapsSymbols", withExtension: "json")!
    let catalogs = try! JSONDecoder().decode([String: SymbolCatalog].self, from: Data(contentsOf: url))
    let sizes: [(CGFloat, UIImage.SymbolWeight, UIImage.SymbolScale, CGFloat, UIUserInterfaceStyle)] = [
        (15, .regular, .medium, 2, .light), (17, .regular, .medium, 3, .light), (20, .regular, .medium, 1, .dark),
        (17.03, .regular, .medium, 3, .light), (29, .bold, .large, 2, .dark), (21, .ultraLight, .small, 1, .light)
    ]
    let masterModes: [(String, UIImage.SymbolConfiguration?, Double?)] = [
        ("default", nil, nil), ("multicolor", .preferringMulticolor(), nil)
    ]
    let modes = masterModes + [
        ("monochrome", .preferringMonochrome(), nil), ("hierarchical", .init(hierarchicalColor: .systemOrange), nil),
        ("palette", .init(paletteColors: [.systemIndigo, .systemYellow, .systemTeal]), nil),
        ("variable 0", .preferringMulticolor(), 0), ("variable 0.75", .preferringMulticolor(), 0.75)
    ]
    let weights: [UIImage.SymbolWeight: String] = [.ultraLight: "Ultralight", .thin: "Thin", .light: "Light",
        .regular: "Regular", .medium: "Medium", .semibold: "Semibold", .bold: "Bold", .heavy: "Heavy", .black: "Black"]
    let scales: [UIImage.SymbolScale: String] = [.small: "Small", .medium: "Medium", .large: "Large"]
    var cases = sizes.map { ($0, modes) }
    for weight in weights.keys.sorted(by: { $0.rawValue < $1.rawValue }) {
        for scale in [UIImage.SymbolScale.small, .medium, .large] where weight != .regular || scale != .medium {
            cases.append(((17, weight, scale, 2, .light), masterModes))
        }
    }
    var comparisons = 0, failures = 0
    for (catalog, linux, native) in [("app", Bundle.main, xcode), ("widget", linuxWidget, xcodeWidget)] {
        let assets = catalogs[catalog]!
        for ((points, weight, size, display, style), sampleModes) in cases {
            let traits = UITraitCollection {
                $0.displayScale = display
                $0.userInterfaceStyle = style
                $0.layoutDirection = .leftToRight
            }
            let base = UIImage.SymbolConfiguration(pointSize: points, weight: weight, scale: size)
            traits.performAsCurrent {
                for name in assets.names {
                    let master = CachedMaster(name: name, weight: weights[weight]!, size: scales[size]!, display: Int(display))
                    let cached = assets.cached.contains(master) && [15, 17, 20].contains(points.rounded())
                    let generated = assets.generated.contains(SymbolMaster(name: name, weight: master.weight, size: master.size))
                    for (mode, colors, variable) in sampleModes {
                        autoreleasepool {
                            let config = (colors.map { base.applying($0) } ?? base).withTraitCollection(traits)
                            let label = "\(catalog)/\(name) \(master.weight)-\(master.size) \(mode), \(points)pt at \(display)x"
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
                            comparisons += 1
                            let reason: String?
                            if test.size != reference.size || test.baselineOffsetFromBottom != reference.baselineOffsetFromBottom
                                || test.alignmentRectInsets != reference.alignmentRectInsets {
                                func layout(_ image: UIImage) -> String {
                                    let baseline = String(describing: image.baselineOffsetFromBottom)
                                    return "\(image.size), baseline \(baseline), insets \(image.alignmentRectInsets)"
                                }
                                reason = "layout differs: Xcode \(layout(reference)), Linux \(layout(test))"
                            } else {
                                reason = SymbolPixels(test, scale: display).difference(from: SymbolPixels(reference, scale: display),
                                    cached: cached, fractional: points != points.rounded(), generated: generated)
                            }
                            if let reason {
                                failures += 1
                                if failures <= 20 { print("\(label): \(reason)"); fflush(stdout) }
                            }
                        }
                    }
                }
            }
        }
    }
    precondition(failures == 0, "\(failures) of \(comparisons) custom symbol renderings differ")
    print("Compared CoMaps symbols in UIKit: \(catalogs["app"]!.names.count) app and \(catalogs["widget"]!.names.count) widget sets, \(comparisons) renderings")
}
