import Cocoa
import Seoyu

public final class WikiDigestCardView: NSView {
    private let summaryLabel = NSTextField(labelWithString: "")
    private let hotStack = NSStackView()

    public override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func setup() {
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor

        summaryLabel.font = .systemFont(ofSize: 12, weight: .semibold)
        summaryLabel.textColor = .secondaryLabelColor
        summaryLabel.translatesAutoresizingMaskIntoConstraints = false

        hotStack.orientation = .horizontal
        hotStack.spacing = 8
        hotStack.translatesAutoresizingMaskIntoConstraints = false

        let root = NSStackView(views: [summaryLabel, hotStack])
        root.orientation = .vertical
        root.alignment = .leading
        root.spacing = 6
        root.translatesAutoresizingMaskIntoConstraints = false
        addSubview(root)
        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            root.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            root.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            root.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])
    }

    public func configure(with digest: WikiDigest?) {
        guard let d = digest, d.topicCount > 0 || d.messageCount > 0 else {
            isHidden = true
            return
        }
        isHidden = false
        summaryLabel.stringValue = "\(d.topicCount) topics · \(d.messageCount) msgs today"
        hotStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for topic in d.hotTopics.prefix(3) {
            let label = NSTextField(labelWithString: topic.title)
            label.font = .systemFont(ofSize: 11)
            label.textColor = .labelColor
            label.lineBreakMode = .byTruncatingTail
            hotStack.addArrangedSubview(label)
        }
    }
}
