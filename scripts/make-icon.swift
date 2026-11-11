// Génère l'icône de ThermoPilot : assets/icon-1024.png puis assets/AppIcon.icns
//   swift scripts/make-icon.swift
import AppKit

let size: CGFloat = 1024
let out = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "assets/icon-1024.png")

func color(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}

let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
                    space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
let c = CGPoint(x: size / 2, y: size / 2)

// ---- Fond : « squircle » macOS (824 px dans 1024, ombre douce)
let inset: CGFloat = 100
let tile = CGRect(x: inset, y: inset, width: size - 2 * inset, height: size - 2 * inset)
let tilePath = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0x000000, 0.35))
ctx.addPath(tilePath)
ctx.setFillColor(color(0x0A2A5E))
ctx.fillPath()
ctx.restoreGState()

ctx.saveGState()
ctx.addPath(tilePath)
ctx.clip()
let bg = CGGradient(colorsSpace: cs, colors: [color(0x0B1F4B), color(0x0E4C8A), color(0x13A4C8)] as CFArray, locations: [0, 0.55, 1])!
ctx.drawLinearGradient(bg, start: CGPoint(x: tile.minX, y: tile.minY), end: CGPoint(x: tile.maxX, y: tile.maxY), options: [])
// reflet en haut
let gloss = CGGradient(colorsSpace: cs, colors: [color(0xFFFFFF, 0.18), color(0xFFFFFF, 0)] as CFArray, locations: [0, 1])!
ctx.drawRadialGradient(gloss, startCenter: CGPoint(x: c.x, y: tile.maxY), startRadius: 0,
                       endCenter: CGPoint(x: c.x, y: tile.maxY), endRadius: 520, options: [])
ctx.restoreGState()

// ---- Jauge thermique : arc de 270° vert -> jaune -> orange -> rouge
let R: CGFloat = 318
let arcWidth: CGFloat = 40
let stops: [(CGFloat, UInt32)] = [(0, 0x34D399), (0.45, 0xFACC15), (0.75, 0xFB923C), (1, 0xEF4444)]
func lerpColor(_ t: CGFloat) -> CGColor {
    var i = 0
    while i < stops.count - 2 && t > stops[i + 1].0 { i += 1 }
    let (t0, c0) = stops[i], (t1, c1) = stops[i + 1]
    let u = max(0, min(1, (t - t0) / (t1 - t0)))
    func ch(_ v: UInt32, _ s: UInt32) -> CGFloat { CGFloat((v >> s) & 0xFF) / 255 }
    return CGColor(red: ch(c0, 16) + (ch(c1, 16) - ch(c0, 16)) * u,
                   green: ch(c0, 8) + (ch(c1, 8) - ch(c0, 8)) * u,
                   blue: ch(c0, 0) + (ch(c1, 0) - ch(c0, 0)) * u, alpha: 1)
}
let start = CGFloat.pi * 1.25, sweep = CGFloat.pi * 1.5
// piste
ctx.setLineCap(.round)
ctx.setLineWidth(arcWidth)
ctx.setStrokeColor(color(0xFFFFFF, 0.12))
ctx.addArc(center: c, radius: R, startAngle: start, endAngle: start - sweep, clockwise: true)
ctx.strokePath()
// dégradé par petits segments (jusqu'à 78 % : « la température monte »)
let filled: CGFloat = 0.78
let n = 240
ctx.setLineCap(.butt)
for k in 0..<n {
    let t0 = CGFloat(k) / CGFloat(n) * filled, t1 = CGFloat(k + 1) / CGFloat(n) * filled + 0.002
    ctx.setStrokeColor(lerpColor(t0 / filled * 0.95))
    ctx.addArc(center: c, radius: R, startAngle: start - sweep * t0, endAngle: start - sweep * t1, clockwise: true)
    ctx.strokePath()
}
// extrémités arrondies + curseur
func dot(_ t: CGFloat, _ r: CGFloat, _ col: CGColor) {
    let a = start - sweep * t
    let p = CGPoint(x: c.x + R * cos(a), y: c.y + R * sin(a))
    ctx.setFillColor(col)
    ctx.fillEllipse(in: CGRect(x: p.x - r, y: p.y - r, width: 2 * r, height: 2 * r))
}
dot(0, arcWidth / 2, lerpColor(0))
dot(filled, arcWidth / 2, lerpColor(0.95))
ctx.saveGState()
ctx.setShadow(offset: .zero, blur: 14, color: color(0x000000, 0.4))
dot(filled, 30, color(0xFFFFFF))
ctx.restoreGState()
dot(filled, 16, lerpColor(0.95))

// ---- Ventilateur : 5 pales incurvées
let bladeR: CGFloat = 238
ctx.saveGState()
ctx.translateBy(x: c.x, y: c.y)
ctx.setShadow(offset: CGSize(width: 0, height: -6), blur: 16, color: color(0x000000, 0.35))
for i in 0..<5 {
    ctx.saveGState()
    ctx.rotate(by: CGFloat(i) * 2 * .pi / 5 + 0.3)
    // pale en faucille : bord d'attaque bombé, bord de fuite creusé
    let p = CGMutablePath()
    p.move(to: CGPoint(x: 0, y: 0))
    p.addCurve(to: CGPoint(x: bladeR * 0.97, y: bladeR * 0.34),
               control1: CGPoint(x: bladeR * 0.10, y: bladeR * 0.58),
               control2: CGPoint(x: bladeR * 0.66, y: bladeR * 0.80))
    p.addQuadCurve(to: CGPoint(x: bladeR * 0.93, y: -bladeR * 0.06),
                   control: CGPoint(x: bladeR * 1.12, y: bladeR * 0.12))
    p.addCurve(to: CGPoint(x: 0, y: 0),
               control1: CGPoint(x: bladeR * 0.62, y: bladeR * 0.22),
               control2: CGPoint(x: bladeR * 0.28, y: -bladeR * 0.02))
    ctx.addPath(p)
    ctx.clip()
    let bl = CGGradient(colorsSpace: cs, colors: [color(0xFFFFFF), color(0xCDEFFF)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(bl, start: .zero, end: CGPoint(x: bladeR, y: bladeR * 0.4), options: [.drawsAfterEndLocation])
    ctx.restoreGState()
}
ctx.restoreGState()

// moyeu
let hub: CGFloat = 62
ctx.setFillColor(color(0xFFFFFF))
ctx.fillEllipse(in: CGRect(x: c.x - hub - 10, y: c.y - hub - 10, width: 2 * (hub + 10), height: 2 * (hub + 10)))
let hubG = CGGradient(colorsSpace: cs, colors: [color(0x1D6FB8), color(0x0B2A5C)] as CFArray, locations: [0, 1])!
ctx.saveGState()
ctx.addEllipse(in: CGRect(x: c.x - hub, y: c.y - hub, width: 2 * hub, height: 2 * hub))
ctx.clip()
ctx.drawLinearGradient(hubG, start: CGPoint(x: c.x, y: c.y + hub), end: CGPoint(x: c.x, y: c.y - hub), options: [])
ctx.restoreGState()
ctx.setFillColor(color(0xFFFFFF, 0.9))
ctx.fillEllipse(in: CGRect(x: c.x - 14, y: c.y - 14, width: 28, height: 28))

// ---- Écriture PNG
let img = ctx.makeImage()!
let rep = NSBitmapImageRep(cgImage: img)
try! FileManager.default.createDirectory(at: out.deletingLastPathComponent(), withIntermediateDirectories: true)
try! rep.representation(using: .png, properties: [:])!.write(to: out)
print("✅ \(out.path)")
