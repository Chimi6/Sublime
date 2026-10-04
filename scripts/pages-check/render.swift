import PDFKit; import AppKit
let args = CommandLine.arguments
guard let doc = PDFDocument(url: URL(fileURLWithPath: args[1])) else { print("no pdf"); exit(1) }
print("pages:", doc.pageCount)
for i in 0..<doc.pageCount {
  let page = doc.page(at: i)!; let b = page.bounds(for: .mediaBox); let s: CGFloat = 1.5
  let img = NSImage(size: NSSize(width: b.width*s, height: b.height*s)); img.lockFocus()
  NSColor.white.set(); NSRect(origin: .zero, size: img.size).fill()
  let ctx = NSGraphicsContext.current!.cgContext; ctx.scaleBy(x: s, y: s); page.draw(with: .mediaBox, to: ctx); img.unlockFocus()
  let rep = NSBitmapImageRep(data: img.tiffRepresentation!)!
  try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(args[2])-p\(i+1).png"))
}
