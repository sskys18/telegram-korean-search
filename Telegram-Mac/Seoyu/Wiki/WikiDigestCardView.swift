import Cocoa
import Seoyu

public final class WikiDigestCardView: NSView {
    private let dateLabel = NSTextField(labelWithString: "")
    private let topicCountLabel = NSTextField(labelWithString: "0")
    private let topicCaptionLabel = NSTextField(labelWithString: "topics")
    private let msgCountLabel = NSTextField(labelWithString: "0")
    private let msgCaptionLabel = NSTextField(labelWithString: "msgs")
    private let hotStack = NSStackView()

    public override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func setup() {
        wantsLayer = true
        layer?.cornerRadius = 10
        layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor

        dateLabel.font = .systemFont(ofSize: 11, weight: .medium)
        dateLabel.textColor = .tertiaryLabelColor

        topicCountLabel.font = .systemFont(ofSize: 22, weight: .bold)
        topicCountLabel.textColor = .controlAccentColor
        topicCaptionLabel.font = .systemFont(ofSize: 11)
        topicCaptionLabel.textColor = .secondaryLabelColor

        msgCountLabel.font = .systemFont(ofSize: 22, weight: .bold)
        msgCountLabel.textColor = .labelColor
        msgCaptionLabel.font = .systemFont(ofSize: 11)
        msgCaptionLabel.textColor = .secondaryLabelColor

        let topicStack = verticalStack(topicCountLabel, topicCaptionLabel)
        let msgStack = verticalStack(msgCountLabel, msgCaptionLabel)

        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = NSColor.separatorColor.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        divider.widthAnchor.constraint(equalToConstant: 1).isActive = true

        let numbersRow = NSStackView(views: [topicStack, divider, msgStack])
        numbersRow.orientation = .horizontal
        numbersRow.spacing = 18
        numbersRow.alignment = .centerY

        hotStack.orientation = .vertical
        hotStack.alignment = .leading
        hotStack.spacing = 3
        hotStack.translatesAutoresizingMaskIntoConstraints = false

        let root = NSStackView(views: [dateLabel, numbersRow, hotStack])
        root.orientation = .vertical
        root.alignment = .leading
        root.spacing = 8
        root.translatesAutoresizingMaskIntoConstraints = false
        addSubview(root)
        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            root.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            root.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            root.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            divider.heightAnchor.constraint(equalToConstant: 28),
        ])
    }

    private func verticalStack(_ a: NSView, _ b: NSView) -> NSStackView {
        let s = NSStackView(views: [a, b])
        s.orientation = .vertical
        s.alignment = .leading
        s.spacing = 0
        return s
    }

    public func configure(with digest: WikiDigest?) {
        guard let d = digest, d.topicCount > 0 || d.messageCount > 0 else {
            isHidden = true
            return
        }
        isHidden = false
        dateLabel.stringValue = "TODAY · \(d.dateYmd)"
        topicCountLabel.stringValue = "\(d.topicCount)"
        msgCountLabel.stringValue = "\(d.messageCount)"
        hotStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for topic in d.hotTopics.prefix(3) {
            let row = NSStackView(views: [
                {
                    let dot = NSTextField(labelWithString: "▸")
                    dot.font = .systemFont(ofSize: 10)
                    dot.textColor = .tertiaryLabelColor
                    return dot
                }(),
                {
                    let label = NSTextField(labelWithString: topic.title)
                    label.font = .systemFont(ofSize: 12, weight: .medium)
                    label.textColor = .labelColor
                    label.lineBreakMode = .byTruncatingTail
                    return label
                }(),
                {
                    let n = NSTextField(labelWithString: "\(topic.messageCount)")
                    n.font = .systemFont(ofSize: 10, weight: .semibold)
                    n.textColor = .tertiaryLabelColor
                    return n
                }(),
            ])
            row.orientation = .horizontal
            row.spacing = 6
            hotStack.addArrangedSubview(row)
        }
    }
}
