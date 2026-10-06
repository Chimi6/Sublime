// cocoa-render <in.rtf> <out.pdf>: lays out an RTF file with the macOS text
// system (as TextEdit does, in page mode: the file's paper size and margins)
// and writes the pages as a PDF. Prints the page count.
import AppKit

let arguments = CommandLine.arguments
guard arguments.count == 3 else {
    FileHandle.standardError.write("usage: cocoa-render <in.rtf> <out.pdf>\n".data(using: .utf8)!)
    exit(4)
}
let input = URL(fileURLWithPath: arguments[1])
var attributes: NSDictionary? = nil
guard let text = try? NSAttributedString(url: input, options: [:], documentAttributes: &attributes) else {
    FileHandle.standardError.write("cannot read \(arguments[1])\n".data(using: .utf8)!)
    exit(1)
}
let document = attributes as? [NSAttributedString.DocumentAttributeKey: Any] ?? [:]
let paper = (document[.paperSize] as? NSValue)?.sizeValue ?? NSSize(width: 612, height: 792)
let left = CGFloat((document[.leftMargin] as? NSNumber)?.doubleValue ?? 72)
let right = CGFloat((document[.rightMargin] as? NSNumber)?.doubleValue ?? 72)
let top = CGFloat((document[.topMargin] as? NSNumber)?.doubleValue ?? 72)
let bottom = CGFloat((document[.bottomMargin] as? NSNumber)?.doubleValue ?? 72)
let size = NSSize(width: paper.width - left - right, height: paper.height - top - bottom)

let storage = NSTextStorage(attributedString: text)
let layout = NSLayoutManager()
storage.addLayoutManager(layout)
var containers: [NSTextContainer] = []
while true {
    let container = NSTextContainer(size: size)
    container.lineFragmentPadding = 0
    layout.addTextContainer(container)
    containers.append(container)
    let range = layout.glyphRange(for: container)
    if NSMaxRange(range) >= layout.numberOfGlyphs || containers.count > 2000 {
        break
    }
}

var box = CGRect(origin: .zero, size: paper)
guard let context = CGContext(URL(fileURLWithPath: arguments[2]) as CFURL, mediaBox: &box, nil) else {
    exit(1)
}
for container in containers {
    context.beginPDFPage(nil)
    let graphics = NSGraphicsContext(cgContext: context, flipped: true)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = graphics
    context.translateBy(x: 0, y: paper.height)
    context.scaleBy(x: 1, y: -1)
    let range = layout.glyphRange(for: container)
    let origin = NSPoint(x: left, y: top)
    layout.drawBackground(forGlyphRange: range, at: origin)
    layout.drawGlyphs(forGlyphRange: range, at: origin)
    NSGraphicsContext.restoreGraphicsState()
    context.endPDFPage()
}
context.closePDF()
print(containers.count)
