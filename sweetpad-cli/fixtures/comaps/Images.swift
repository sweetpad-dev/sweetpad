import SwiftUI
import UIKit

// The ten expressions used in the pinned CoMaps application sources.
@MainActor func checkCoMapsImages() {
    let images = [UIImage(resource: .icWalk), UIImage(resource: .ic24PxEdit), UIImage(resource: .icSearch)]
    precondition(images.allSatisfy { $0.size.width > 0 && $0.size.height > 0 })
    let light = UITraitCollection(userInterfaceStyle: .light)
    for color in [UIColor(resource: .accent), UIColor(Color(.accent))] {
        precondition(color.resolvedColor(with: light).cgColor.alpha > 0, "Empty Accent Color resource")
    }
    let swiftUIImages = [
        Image(.openStreetMapLogo), Image(.Brands.codeberg), Image(.Brands.mastodon),
        Image(.Brands.matrix), Image(.Brands.lemmy), Image(.Brands.bluesky), Image(.Brands.pixelfed),
    ]
    if #available(iOS 16.0, *) {
        for image in swiftUIImages {
            let renderer = ImageRenderer(content: image.resizable().frame(width: 32, height: 32))
            guard let raster = renderer.cgImage else { fatalError("SwiftUI could not render a CoMaps image") }
            guard let context = CGContext(data: nil, width: 32, height: 32, bitsPerComponent: 8, bytesPerRow: 32 * 4,
                                          space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { fatalError("Could not create an image mask") }
            context.draw(raster, in: CGRect(x: 0, y: 0, width: 32, height: 32))
            let pixels = context.data!.assumingMemoryBound(to: UInt8.self)
            precondition(stride(from: 3, to: 32 * 32 * 4, by: 4).contains { pixels[$0] != 0 }, "SwiftUI rendered an empty CoMaps image")
        }
    }
}
