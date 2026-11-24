// Makes the phone's app icon out of the Mac's.
//
//   swift ios/icon.swift <the Mac's icon, 1024 px> <where to write the phone's>
//
// The Mac's icon is drawn the way macOS wants one: the picture is a rounded
// square 824 px across with a clear margin around it. iOS wants the opposite —
// the whole square filled, nothing see-through, and it rounds the corners
// itself. So the picture is cut out, grown to fill the square, and laid on the
// colour of its own background, which is what shows in the corners until iOS
// trims them off.

import AppKit

let arguments = CommandLine.arguments
guard arguments.count == 3,
      let source = NSImage(contentsOfFile: arguments[1])?.cgImage(forProposedRect: nil, context: nil, hints: nil)
else {
    print("usage: swift ios/icon.swift <1024 px icon> <output png>")
    exit(2)
}

let side = 1024
// The body is 824 of the 1024, wherever the source was rendered at.
let margin = CGFloat(source.width) * 100 / 1024
let body = CGRect(
    x: margin,
    y: margin,
    width: CGFloat(source.width) - 2 * margin,
    height: CGFloat(source.height) - 2 * margin
)

guard let cut = source.cropping(to: body),
      let canvas = CGContext(
          data: nil,
          width: side,
          height: side,
          bitsPerComponent: 8,
          bytesPerRow: 0,
          space: CGColorSpace(name: CGColorSpace.sRGB)!,
          // No alpha at all: an icon with any is refused by the App Store and
          // drawn on black by the phone.
          bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
      )
else {
    print("could not draw \(arguments[1])")
    exit(1)
}

// The middle of the icon's own gradient (`#7c1a31` in assets/icon.svg).
canvas.setFillColor(CGColor(srgbRed: 0x7c / 255, green: 0x1a / 255, blue: 0x31 / 255, alpha: 1))
canvas.fill(CGRect(x: 0, y: 0, width: side, height: side))
canvas.interpolationQuality = .high
canvas.draw(cut, in: CGRect(x: 0, y: 0, width: side, height: side))

guard let image = canvas.makeImage(),
      let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
else {
    print("could not make a picture of it")
    exit(1)
}
try png.write(to: URL(fileURLWithPath: arguments[2]))
print("wrote \(arguments[2])")
