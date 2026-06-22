//
//  Rendering.swift
//
//  Headless rasterization of a discovered preview via SwiftUI's ImageRenderer
//  (macOS 13+), plus average-color sampling. Lets the regression suite render a
//  preview and assert it produced the expected view — no simulator required.

import CoreGraphics
import SwiftUI

public struct AverageColor: Equatable {
  public let r: UInt8
  public let g: UInt8
  public let b: UInt8
  public let a: UInt8
}

/// Render a view to a CGImage off-screen. Returns nil if rasterization fails.
@MainActor
public func renderCGImage(_ view: any View, scale: CGFloat = 1) -> CGImage? {
  let renderer = ImageRenderer(content: AnyView(view))
  renderer.scale = scale
  return renderer.cgImage
}

/// Render a view and return its average pixel color, by drawing the rendered
/// image into a 1×1 sRGB context. For a solid-fill view this equals the fill.
@MainActor
public func renderAverageColor(_ view: any View) -> AverageColor? {
  guard let image = renderCGImage(view) else { return nil }
  return averageColor(of: image)
}

public func averageColor(of image: CGImage) -> AverageColor? {
  guard let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
  var pixel = [UInt8](repeating: 0, count: 4)
  let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue
  // Draw while the buffer pointer is valid (drawing the whole image into a 1×1
  // box averages every pixel), then read the bytes back out.
  let drawn = pixel.withUnsafeMutableBytes { bytes -> Bool in
    guard let context = CGContext(
      data: bytes.baseAddress,
      width: 1,
      height: 1,
      bitsPerComponent: 8,
      bytesPerRow: 4,
      space: space,
      bitmapInfo: bitmapInfo,
    ) else {
      return false
    }
    context.interpolationQuality = .none
    context.draw(image, in: CGRect(x: 0, y: 0, width: 1, height: 1))
    return true
  }
  guard drawn else { return nil }
  return AverageColor(r: pixel[0], g: pixel[1], b: pixel[2], a: pixel[3])
}
