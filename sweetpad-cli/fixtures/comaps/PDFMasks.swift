import UIKit

private struct AlphaRaster {
    let width: Int
    let height: Int
    let pixels: [UInt8]

    @MainActor init(_ name: String, in bundle: Bundle, traits: UITraitCollection) {
        guard let image = UIImage(named: name, in: bundle, compatibleWith: traits),
              let raster = image.cgImage, image.scale == traits.displayScale else {
            fatalError("Missing \(name) at \(traits.displayScale)x in \(bundle.bundleURL.lastPathComponent)")
        }
        width = raster.width
        height = raster.height
        let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.setBlendMode(.copy)
        context.draw(raster, in: CGRect(x: 0, y: 0, width: width, height: height))
        let rgba = context.data!.assumingMemoryBound(to: UInt8.self)
        pixels = (0..<(width * height)).map { rgba[$0 * 4 + 3] }
    }

    subscript(x: Int, y: Int) -> UInt8 { pixels[y * width + x] }

    func checkInterior(against reference: AlphaRaster, name: String, scale: Int) {
        precondition(width == reference.width && height == reference.height, "\(name) has different dimensions at \(scale)x")
        precondition(pixels.contains { $0 > 0 } && reference.pixels.contains { $0 > 0 },
                     "\(name) rendered empty at \(scale)x")
        guard width > 2 && height > 2 else { return }
        let neighbors = [-width - 1, -width, -width + 1, -1, 1, width - 1, width, width + 1]
        // Core Graphics and Hayro can antialias contours differently. Sample solid and
        // clear interiors at least one pixel away from a change in the reference mask.
        for y in 1..<(height - 1) {
            for x in 1..<(width - 1) {
                let i = y * width + x, expected = reference.pixels[i]
                if (expected == 0 || expected == 255) && abs(Int(expected) - Int(pixels[i])) > 8
                    && neighbors.allSatisfy({ reference.pixels[i + $0] == expected }) {
                    preconditionFailure("\(name) at \(scale)x (\(x), \(y)): expected interior alpha \(expected), got \(pixels[i])")
                }
            }
        }
    }
}

@available(iOS 17.0, *)
@MainActor func checkPDFMasks(linuxMasks: Bundle, xcodeMasks: Bundle, xcodeCoMaps: Bundle) {
    let list = Bundle.main.url(forResource: "CoMapsPDFs", withExtension: "json")!
    let names = try! JSONDecoder().decode([String].self, from: Data(contentsOf: list))
    for scale in 1...3 {
        let traits = UITraitCollection {
            $0.displayScale = CGFloat(scale)
            $0.layoutDirection = .leftToRight
            $0.userInterfaceStyle = .light
        }
        for bundle in [xcodeMasks, linuxMasks] {
            let mask = AlphaRaster("AlphaCutout", in: bundle, traits: traits)
            precondition(mask.width == 16 * scale && mask.height == 12 * scale, "Wrong AlphaCutout dimensions at \(scale)x")
            precondition(mask[0, 0] == 0 && mask[3 * scale, 3 * scale] == 255 && mask[6 * scale, 6 * scale] == 0,
                         "Lost the AlphaCutout mask at \(scale)x in \(bundle.bundleURL.lastPathComponent)")
            precondition(abs(Int(mask[8 * scale, 6 * scale]) - 128) <= 1, "Wrong partial alpha in \(bundle)")
        }
        for name in names {
            autoreleasepool {
                let reference = AlphaRaster(name, in: xcodeCoMaps, traits: traits)
                let linux = AlphaRaster(name, in: .main, traits: traits)
                linux.checkInterior(against: reference, name: name, scale: scale)
                if name == "ic_booking_plus" || name == "ic_booking_minus" {
                    for mask in [reference, linux] {
                        precondition(mask.width == 22 * scale && mask.height == 22 * scale, "Wrong \(name) dimensions")
                        precondition(mask[0, 0] == 0 && mask[11 * scale, 11 * scale] == 0
                                     && mask[3 * scale, 11 * scale] == 255, "Lost the \(name) ring at \(scale)x")
                        precondition(mask[11 * scale, 7 * scale] == (name == "ic_booking_plus" ? 0 : 255),
                                     "Lost the \(name) cutout at \(scale)x")
                    }
                }
            }
        }
    }
}
