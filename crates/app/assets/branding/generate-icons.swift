#!/usr/bin/env swift
import AppKit
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Regenerates Terminator app icons with Apple's continuous-corner (squircle) mask.
///
/// Usage (from repo root):
///   swift crates/app/assets/branding/generate-icons.swift
///
/// Reads the unmasked Red Eye artwork and writes terminator.png, terminator-512.png,
/// and terminator.icns into this directory.

let cornerRadiusRatio = 0.225

struct Paths {
    let source: URL
    let branding: URL
    let png: URL
    let png512: URL
    let icns: URL

    static func resolve() throws -> Paths {
        let file = URL(fileURLWithPath: #filePath).standardizedFileURL
        let branding = file.deletingLastPathComponent()
        let source = branding.appendingPathComponent("red-eye-source.png")
        return Paths(
            source: source,
            branding: branding,
            png: branding.appendingPathComponent("terminator.png"),
            png512: branding.appendingPathComponent("terminator-512.png"),
            icns: branding.appendingPathComponent("terminator.icns")
        )
    }
}

func die(_ message: String) -> Never {
    FileHandle.standardError.write(Data("\(message)\n".utf8))
    exit(1)
}

func loadCGImage(_ url: URL) -> CGImage {
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else {
        die("Failed to load \(url.path)")
    }
    return image
}

func writePNG(_ image: CGImage, to url: URL) {
    guard let dest = CGImageDestinationCreateWithURL(
        url as CFURL,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else {
        die("Failed to create PNG writer for \(url.path)")
    }
    CGImageDestinationAddImage(dest, image, nil)
    guard CGImageDestinationFinalize(dest) else {
        die("Failed to write \(url.path)")
    }
}

func point(x: CGFloat, y: CGFloat, radius: CGFloat, size: CGFloat, quadrant: String) -> CGPoint {
    var px = x * radius
    var py = y * radius
    if quadrant == "topRight" || quadrant == "bottomRight" {
        px = size - px
    }
    if quadrant == "bottomLeft" || quadrant == "bottomRight" {
        py = size - py
    }
    return CGPoint(x: px, y: py)
}

func addCurve(
    _ path: CGMutablePath,
    to: (CGFloat, CGFloat),
    control1: (CGFloat, CGFloat),
    control2: (CGFloat, CGFloat),
    radius: CGFloat,
    size: CGFloat,
    quadrant: String
) {
    path.addCurve(
        to: point(x: to.0, y: to.1, radius: radius, size: size, quadrant: quadrant),
        control1: point(x: control1.0, y: control1.1, radius: radius, size: size, quadrant: quadrant),
        control2: point(x: control2.0, y: control2.1, radius: radius, size: size, quadrant: quadrant)
    )
}

func continuousRoundedPath(size: CGFloat) -> CGPath {
    let radius = size * cornerRadiusRatio
    let path = CGMutablePath()

    path.move(to: point(x: 1.528665, y: 0, radius: radius, size: size, quadrant: "topLeft"))
    path.addLine(to: point(x: 1.528665, y: 0, radius: radius, size: size, quadrant: "topRight"))
    addCurve(
        path,
        to: (0.63149379, 0.07491139),
        control1: (1.08849296, 0),
        control2: (0.86840694, 0),
        radius: radius,
        size: size,
        quadrant: "topRight"
    )
    addCurve(
        path,
        to: (0.07491139, 0.63149379),
        control1: (0.37282383, 0.16905956),
        control2: (0.16905956, 0.37282383),
        radius: radius,
        size: size,
        quadrant: "topRight"
    )
    addCurve(
        path,
        to: (0, 1.52866498),
        control1: (0, 0.86840694),
        control2: (0, 1.08849296),
        radius: radius,
        size: size,
        quadrant: "topRight"
    )

    path.addLine(to: point(x: 0, y: 1.528665, radius: radius, size: size, quadrant: "bottomRight"))
    addCurve(
        path,
        to: (0.07491139, 0.63149379),
        control1: (0, 1.08849296),
        control2: (0, 0.86840694),
        radius: radius,
        size: size,
        quadrant: "bottomRight"
    )
    addCurve(
        path,
        to: (0.63149379, 0.07491139),
        control1: (0.16905956, 0.37282383),
        control2: (0.37282383, 0.16905956),
        radius: radius,
        size: size,
        quadrant: "bottomRight"
    )
    addCurve(
        path,
        to: (1.52866498, 0),
        control1: (0.86840694, 0),
        control2: (1.08849296, 0),
        radius: radius,
        size: size,
        quadrant: "bottomRight"
    )

    path.addLine(to: point(x: 1.528665, y: 0, radius: radius, size: size, quadrant: "bottomLeft"))
    addCurve(
        path,
        to: (0.63149379, 0.07491139),
        control1: (1.08849296, 0),
        control2: (0.86840694, 0),
        radius: radius,
        size: size,
        quadrant: "bottomLeft"
    )
    addCurve(
        path,
        to: (0.07491139, 0.63149379),
        control1: (0.37282383, 0.16905956),
        control2: (0.16905956, 0.37282383),
        radius: radius,
        size: size,
        quadrant: "bottomLeft"
    )
    addCurve(
        path,
        to: (0, 1.52866498),
        control1: (0, 0.86840694),
        control2: (0, 1.08849296),
        radius: radius,
        size: size,
        quadrant: "bottomLeft"
    )

    path.addLine(to: point(x: 0, y: 1.528665, radius: radius, size: size, quadrant: "topLeft"))
    addCurve(
        path,
        to: (0.07491139, 0.63149379),
        control1: (0, 1.08849296),
        control2: (0, 0.86840694),
        radius: radius,
        size: size,
        quadrant: "topLeft"
    )
    addCurve(
        path,
        to: (0.63149379, 0.07491139),
        control1: (0.16905956, 0.37282383),
        control2: (0.37282383, 0.16905956),
        radius: radius,
        size: size,
        quadrant: "topLeft"
    )
    addCurve(
        path,
        to: (1.52866498, 0),
        control1: (0.86840694, 0),
        control2: (1.08849296, 0),
        radius: radius,
        size: size,
        quadrant: "topLeft"
    )
    path.closeSubpath()
    return path
}

func masked(_ source: CGImage) -> CGImage {
    let width = source.width
    let height = source.height
    guard width == height else {
        die("Source icon must be square, got \(width)x\(height)")
    }
    let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    guard let ctx = CGContext(
        data: nil,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        die("Failed to create bitmap context")
    }
    ctx.setAllowsAntialiasing(true)
    ctx.setShouldAntialias(true)
    ctx.translateBy(x: 0, y: CGFloat(height))
    ctx.scaleBy(x: 1, y: -1)
    ctx.addPath(continuousRoundedPath(size: CGFloat(width)))
    ctx.clip()
    ctx.draw(source, in: CGRect(x: 0, y: 0, width: width, height: height))
    guard let image = ctx.makeImage() else {
        die("Failed to rasterize masked icon")
    }
    return image
}

func resized(_ source: CGImage, to pixelSize: Int) -> CGImage {
    let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    guard let ctx = CGContext(
        data: nil,
        width: pixelSize,
        height: pixelSize,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        die("Failed to create resize context")
    }
    ctx.interpolationQuality = .high
    ctx.setAllowsAntialiasing(true)
    ctx.setShouldAntialias(true)
    ctx.translateBy(x: 0, y: CGFloat(pixelSize))
    ctx.scaleBy(x: 1, y: -1)
    ctx.draw(source, in: CGRect(x: 0, y: 0, width: pixelSize, height: pixelSize))
    guard let image = ctx.makeImage() else {
        die("Failed to resize icon to \(pixelSize)")
    }
    return image
}

func writeIconset(from master: CGImage, to icns: URL) throws {
    let temp = FileManager.default.temporaryDirectory
        .appendingPathComponent("terminator-\(UUID().uuidString).iconset", isDirectory: true)
    if FileManager.default.fileExists(atPath: temp.path) {
        try FileManager.default.removeItem(at: temp)
    }
    try FileManager.default.createDirectory(at: temp, withIntermediateDirectories: true)

    let names: [(String, Int)] = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ]
    for (name, size) in names {
        writePNG(resized(master, to: size), to: temp.appendingPathComponent(name))
    }

    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
    process.arguments = ["-c", "icns", "-o", icns.path, temp.path]
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        die("iconutil failed with status \(process.terminationStatus)")
    }
    try FileManager.default.removeItem(at: temp)
}

let paths = try Paths.resolve()
guard FileManager.default.fileExists(atPath: paths.source.path) else {
    die("Missing source artwork: \(paths.source.path)")
}

let master = masked(loadCGImage(paths.source))
writePNG(master, to: paths.png)
writePNG(resized(master, to: 512), to: paths.png512)
try writeIconset(from: master, to: paths.icns)
print("Wrote \(paths.png.path)")
print("Wrote \(paths.png512.path)")
print("Wrote \(paths.icns.path)")
