import Cocoa

public enum MarkdownRenderer {
    public static func render(_ source: String) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let lines = source.components(separatedBy: "\n")
        var paragraph: [String] = []

        func flushParagraph() {
            guard !paragraph.isEmpty else { return }
            let joined = paragraph.joined(separator: " ")
            out.append(renderInline(joined, base: bodyAttrs))
            out.append(NSAttributedString(string: "\n\n"))
            paragraph.removeAll()
        }

        for raw in lines {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty {
                flushParagraph()
                continue
            }
            if line.hasPrefix("## ") {
                flushParagraph()
                let text = String(line.dropFirst(3))
                out.append(renderInline(text, base: h2Attrs))
                out.append(NSAttributedString(string: "\n"))
            } else if line.hasPrefix("# ") {
                flushParagraph()
                let text = String(line.dropFirst(2))
                out.append(renderInline(text, base: h1Attrs))
                out.append(NSAttributedString(string: "\n"))
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                flushParagraph()
                let text = String(line.dropFirst(2))
                let bullet = NSMutableAttributedString(string: "• ", attributes: bodyAttrs)
                bullet.append(renderInline(text, base: bodyAttrs))
                bullet.append(NSAttributedString(string: "\n"))
                out.append(bullet)
            } else {
                paragraph.append(line)
            }
        }
        flushParagraph()
        return out
    }

    private static let bodyAttrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: 13),
        .foregroundColor: NSColor.labelColor,
    ]
    private static let h1Attrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: 20, weight: .bold),
        .foregroundColor: NSColor.labelColor,
    ]
    private static let h2Attrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: 16, weight: .semibold),
        .foregroundColor: NSColor.labelColor,
    ]

    private static func renderInline(
        _ text: String,
        base: [NSAttributedString.Key: Any]
    ) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let chars = Array(text)
        var i = 0
        var buffer = ""

        func flushBuffer() {
            if !buffer.isEmpty {
                out.append(NSAttributedString(string: buffer, attributes: base))
                buffer = ""
            }
        }

        while i < chars.count {
            let c = chars[i]
            if c == "*", i + 1 < chars.count, chars[i + 1] == "*" {
                if let end = findClosing(chars, start: i + 2, marker: "**") {
                    flushBuffer()
                    let inner = String(chars[(i + 2)..<end])
                    var attrs = base
                    let baseFont = (base[.font] as? NSFont) ?? NSFont.systemFont(ofSize: 13)
                    attrs[.font] = NSFontManager.shared.convert(baseFont, toHaveTrait: .boldFontMask)
                    out.append(NSAttributedString(string: inner, attributes: attrs))
                    i = end + 2
                    continue
                }
            } else if c == "*" {
                if let end = findClosing(chars, start: i + 1, marker: "*") {
                    flushBuffer()
                    let inner = String(chars[(i + 1)..<end])
                    var attrs = base
                    let baseFont = (base[.font] as? NSFont) ?? NSFont.systemFont(ofSize: 13)
                    attrs[.font] = NSFontManager.shared.convert(baseFont, toHaveTrait: .italicFontMask)
                    out.append(NSAttributedString(string: inner, attributes: attrs))
                    i = end + 1
                    continue
                }
            } else if c == "`" {
                if let end = findClosing(chars, start: i + 1, marker: "`") {
                    flushBuffer()
                    let inner = String(chars[(i + 1)..<end])
                    var attrs = base
                    attrs[.font] = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
                    attrs[.backgroundColor] = NSColor.controlBackgroundColor
                    out.append(NSAttributedString(string: inner, attributes: attrs))
                    i = end + 1
                    continue
                }
            }
            buffer.append(c)
            i += 1
        }
        flushBuffer()
        return out
    }

    private static func findClosing(_ chars: [Character], start: Int, marker: String) -> Int? {
        let m = Array(marker)
        var i = start
        while i + m.count <= chars.count {
            var match = true
            for k in 0..<m.count where chars[i + k] != m[k] {
                match = false
                break
            }
            if match { return i }
            i += 1
        }
        return nil
    }
}
