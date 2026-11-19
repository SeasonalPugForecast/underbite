// Rewrites each PNG named on the command line without its alpha channel, keeping its pixels
// and colour space: App Store Connect refuses screenshots that have one, and the simulator's
// always do (every pixel is opaque). `cargo xtask ios remote store-shots` runs it on the Mac:
//   xcrun swift ios/shots/store/drop-alpha.swift file.png...
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

for path in CommandLine.arguments.dropFirst() {
    let url = URL(fileURLWithPath: path)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { fail("\(path): not a readable image") }
    let space = image.colorSpace ?? CGColorSpace(name: CGColorSpace.sRGB)!
    guard let context = CGContext(
        data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
        bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
    else { fail("\(path): no drawing context") }
    let bounds = CGRect(x: 0, y: 0, width: image.width, height: image.height)
    context.setFillColor(CGColor(gray: 1, alpha: 1))
    context.fill(bounds)
    context.draw(image, in: bounds)
    guard let flat = context.makeImage() else { fail("\(path): no image made") }
    let temporary = url.deletingLastPathComponent().appendingPathComponent(".flat-\(url.lastPathComponent)")
    guard let destination = CGImageDestinationCreateWithURL(
        temporary as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { fail("\(path): cannot write") }
    CGImageDestinationAddImage(destination, flat, nil)
    guard CGImageDestinationFinalize(destination) else { fail("\(path): cannot write") }
    _ = try? FileManager.default.replaceItemAt(url, withItemAt: temporary)
}
