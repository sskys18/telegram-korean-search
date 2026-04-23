import Cocoa
import Seoyu

public final class WikiCategoryChipsView: NSView {
    private let stack = NSStackView()
    private let overflowButton = NSButton()
    private var allCategories: [WikiCategory] = []
    private var selectedName: String? = nil
    private var expanded = false

    public var onCategorySelected: ((String?) -> Void)?

    public override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func setup() {
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false

        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasHorizontalScroller = false
        scroll.hasVerticalScroller = false
        scroll.drawsBackground = false
        let doc = NSView()
        doc.translatesAutoresizingMaskIntoConstraints = false
        doc.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: doc.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: doc.trailingAnchor),
            stack.topAnchor.constraint(equalTo: doc.topAnchor),
            stack.bottomAnchor.constraint(equalTo: doc.bottomAnchor),
        ])
        scroll.documentView = doc

        addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: topAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            heightAnchor.constraint(equalToConstant: 28),
        ])

        overflowButton.bezelStyle = .inline
        overflowButton.title = "…"
        overflowButton.target = self
        overflowButton.action = #selector(toggleOverflow)
    }

    public func configure(with categories: [WikiCategory], selected: String?) {
        self.allCategories = categories
        self.selectedName = selected
        rebuild()
    }

    private func rebuild() {
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        stack.addArrangedSubview(makeChip(title: "All", value: nil, selected: selectedName == nil))

        let visible = expanded ? allCategories : Array(allCategories.prefix(6))
        for cat in visible {
            let label = displayName(cat)
            stack.addArrangedSubview(makeChip(title: label, value: cat.name, selected: selectedName == cat.name))
        }
        if !expanded && allCategories.count > 6 {
            stack.addArrangedSubview(overflowButton)
        }
    }

    private func displayName(_ cat: WikiCategory) -> String {
        if WikiLocale.current == .ko, let ko = cat.nameKo, !ko.isEmpty {
            return ko
        }
        return cat.name
    }

    private func makeChip(title: String, value: String?, selected: Bool) -> NSButton {
        let btn = NSButton(title: title, target: self, action: #selector(chipClicked(_:)))
        btn.bezelStyle = .inline
        btn.setButtonType(.momentaryPushIn)
        btn.contentTintColor = selected ? .controlAccentColor : .labelColor
        btn.identifier = .init(value ?? "__all__")
        return btn
    }

    @objc private func chipClicked(_ sender: NSButton) {
        let raw = sender.identifier?.rawValue
        let value: String? = (raw == "__all__") ? nil : raw
        selectedName = value
        rebuild()
        onCategorySelected?(value)
    }

    @objc private func toggleOverflow() {
        expanded = true
        rebuild()
    }
}
