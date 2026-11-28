import UIKit

private struct AppearanceResources: Decodable {
    let colors: [String]
    let images: [String]
}

private struct Raster {
    let width: Int
    let height: Int
    let bytes: [UInt8]

    @MainActor init(_ name: String, in bundle: Bundle, traits: UITraitCollection) {
        guard let image = UIImage(named: name, in: bundle, compatibleWith: traits), let raster = image.cgImage,
              image.scale == traits.displayScale else { fatalError("Missing \(name) for \(traits) in \(bundle)") }
        width = raster.width
        height = raster.height
        let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.draw(raster, in: CGRect(x: 0, y: 0, width: width, height: height))
        bytes = Array(UnsafeBufferPointer(start: context.data!.assumingMemoryBound(to: UInt8.self), count: width * height * 4))
        precondition(stride(from: 3, to: bytes.count, by: 4).contains { bytes[$0] > 0 }, "Empty \(name) in \(bundle)")
    }

    func check(against reference: Raster, name: String) {
        precondition(width == reference.width && height == reference.height, "Wrong dimensions for \(name)")
        guard bytes != reference.bytes else { return }
        guard width > 2 && height > 2 else {
            precondition(zip(bytes, reference.bytes).allSatisfy { abs(Int($0.0) - Int($0.1)) <= 2 }, name)
            return
        }
        // Compare the authored fills and transparent regions. Core Graphics and resvg antialias edges differently.
        // Check in both directions so added and missing solid regions both fail.
        let neighbors = [-width - 1, -width, -width + 1, -1, 1, width - 1, width, width + 1]
        for y in 1..<(height - 1) {
            for x in 1..<(width - 1) {
                let i = (y * width + x) * 4
                for channel in 0..<4 {
                    let j = i + channel
                    if abs(Int(bytes[j]) - Int(reference.bytes[j])) <= 2 { continue }
                    let flat = [bytes, reference.bytes].contains { data in
                        neighbors.allSatisfy { data[j + $0 * 4] == data[j] }
                    }
                    precondition(!flat, "\(name) at (\(x), \(y)), channel \(channel): Xcode \(reference.bytes[j]), Linux \(bytes[j])")
                }
            }
        }
    }
}

@available(iOS 17.0, *)
@MainActor func checkCoMapsAppearances(xcode: Bundle) {
    let url = Bundle.main.url(forResource: "CoMapsAppearances", withExtension: "json")!
    let resources = try! JSONDecoder().decode(AppearanceResources.self, from: Data(contentsOf: url))
    for style: UIUserInterfaceStyle in [.light, .dark] {
        for scale: CGFloat in [1, 2, 3] {
            let traits = UITraitCollection { $0.userInterfaceStyle = style; $0.displayScale = scale }
            for name in resources.colors {
                guard let expected = UIColor(named: name, in: xcode, compatibleWith: traits),
                      let actual = UIColor(named: name, in: .main, compatibleWith: traits),
                      actual.resolvedColor(with: traits).cgColor == expected.resolvedColor(with: traits).cgColor else {
                    fatalError("Different CoMaps color \(name) for \(traits)")
                }
            }
            for name in resources.images {
                autoreleasepool {
                    let expected = Raster(name, in: xcode, traits: traits)
                    Raster(name, in: .main, traits: traits).check(against: expected, name: "\(name) \(style.rawValue) at \(scale)x")
                }
            }
        }
    }
    print("Compared CoMaps appearances: \(resources.images.count) SVG sets and \(resources.colors.count) colors in light and dark at 1x, 2x and 3x")
}
