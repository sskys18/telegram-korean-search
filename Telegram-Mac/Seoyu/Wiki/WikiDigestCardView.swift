import Cocoa
import Seoyu

public final class WikiDigestCardView: NSView {
    private let dateLabel = NSTextField(labelWithString: "")
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

        hotStack.orientation = .vertical
        hotStack.alignment = .leading
        hotStack.spacing = 3
        hotStack.translatesAutoresizingMaskIntoConstraints = false

        let root = NSStackView(views: [dateLabel, hotStack])
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
        ])
    }

    public func configure(with digest: WikiDigest?) {
        guard let d = digest, !d.hotTopics.isEmpty else {
            isHidden = true
            return
        }
        isHidden = false
        dateLabel.stringValue = "TODAY · \(d.dateYmd)"
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
