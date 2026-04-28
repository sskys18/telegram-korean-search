import Cocoa
import Seoyu

public final class WikiListViewController: NSViewController,
    NSTableViewDataSource, NSTableViewDelegate
{
    private let seoyu: Seoyu
    private let tableView = NSTableView()
    private let digestView = WikiDigestCardView()
    private let emptyLabel = NSTextField(labelWithString: "")
    private let errorBanner = NSView()
    private let errorLabel = NSTextField(labelWithString: "")
    private let toastLabel = NSTextField(labelWithString: "")
    private let headerLabel = NSTextField(labelWithString: "Trending")

    private var topics: [WikiTopicSummary] = []
    private var seedTopics: [WikiTopicSummary]?
    private var isSearchMode = false
    private var trendingMode = false

    public var onTopicSelected: ((WikiTopicSummary) -> Void)?

    public init(seoyu: Seoyu, seed: [WikiTopicSummary]? = nil) {
        self.seoyu = seoyu
        self.seedTopics = seed
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    public override func loadView() {
        let root = NSStackView()
        root.orientation = .vertical
        root.spacing = 8
        root.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 0, right: 8)
        root.translatesAutoresizingMaskIntoConstraints = false
        root.detachesHiddenViews = true
        root.distribution = .fill
        root.alignment = .leading

        headerLabel.font = .systemFont(ofSize: 18, weight: .bold)
        headerLabel.textColor = .labelColor
        headerLabel.isHidden = true
        root.addArrangedSubview(headerLabel)
        digestView.isHidden = true
        root.addArrangedSubview(digestView)

        let column = NSTableColumn(identifier: .init("topic"))
        column.width = 400
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 68
        tableView.intercellSpacing = NSSize(width: 0, height: 6)
        tableView.selectionHighlightStyle = .regular
        tableView.gridStyleMask = []
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.action = #selector(onRowClicked)

        let scroll = NSScrollView()
        scroll.documentView = tableView
        scroll.hasVerticalScroller = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        root.addArrangedSubview(scroll)
        scroll.widthAnchor.constraint(equalTo: root.widthAnchor).isActive = true
        headerLabel.setContentHuggingPriority(.required, for: .vertical)
        headerLabel.setContentCompressionResistancePriority(.required, for: .vertical)
        digestView.setContentHuggingPriority(.required, for: .vertical)

        emptyLabel.alignment = .center
        emptyLabel.textColor = .secondaryLabelColor
        emptyLabel.isHidden = true
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false

        errorBanner.wantsLayer = true
        errorBanner.layer?.backgroundColor = NSColor.systemRed.withAlphaComponent(0.15).cgColor
        errorBanner.translatesAutoresizingMaskIntoConstraints = false
        errorBanner.isHidden = true
        errorLabel.translatesAutoresizingMaskIntoConstraints = false
        errorLabel.textColor = .labelColor
        errorLabel.lineBreakMode = .byTruncatingTail
        let dismiss = NSButton(title: "Dismiss", target: self, action: #selector(dismissError))
        dismiss.bezelStyle = .inline
        dismiss.translatesAutoresizingMaskIntoConstraints = false
        errorBanner.addSubview(errorLabel)
        errorBanner.addSubview(dismiss)
        NSLayoutConstraint.activate([
            errorLabel.leadingAnchor.constraint(equalTo: errorBanner.leadingAnchor, constant: 12),
            errorLabel.centerYAnchor.constraint(equalTo: errorBanner.centerYAnchor),
            dismiss.trailingAnchor.constraint(equalTo: errorBanner.trailingAnchor, constant: -8),
            dismiss.centerYAnchor.constraint(equalTo: errorBanner.centerYAnchor),
            errorLabel.trailingAnchor.constraint(lessThanOrEqualTo: dismiss.leadingAnchor, constant: -8),
            errorBanner.heightAnchor.constraint(equalToConstant: 32),
        ])

        toastLabel.wantsLayer = true
        toastLabel.layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor
        toastLabel.layer?.cornerRadius = 6
        toastLabel.alignment = .center
        toastLabel.textColor = .labelColor
        toastLabel.translatesAutoresizingMaskIntoConstraints = false
        toastLabel.isHidden = true
        toastLabel.usesSingleLineMode = false
        toastLabel.maximumNumberOfLines = 2

        let container = NSView()
        container.addSubview(errorBanner)
        container.addSubview(root)
        container.addSubview(emptyLabel)
        container.addSubview(toastLabel)
        NSLayoutConstraint.activate([
            errorBanner.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            errorBanner.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            errorBanner.topAnchor.constraint(equalTo: container.topAnchor),
            root.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            root.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            root.topAnchor.constraint(equalTo: errorBanner.bottomAnchor),
            root.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            emptyLabel.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: container.centerYAnchor),
            toastLabel.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            toastLabel.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -16),
            toastLabel.widthAnchor.constraint(lessThanOrEqualTo: container.widthAnchor, multiplier: 0.8),
        ])
        self.view = container

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiTopicsChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.throttledReload() }

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiLanguageChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.tableView.reloadData() }

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiProgress,
            object: nil,
            queue: .main
        ) { [weak self] note in self?.handleProgress(note) }

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiError,
            object: nil,
            queue: .main
        ) { [weak self] note in self?.handleError(note) }
    }

    private func handleError(_ note: Notification) {
        let message = (note.userInfo?["message"] as? String) ?? "wiki error"
        let recoverable = (note.userInfo?["recoverable"] as? Bool) ?? true
        if recoverable {
            toastLabel.stringValue = "  \(message)  "
            toastLabel.isHidden = false
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                self?.toastLabel.isHidden = true
            }
        } else {
            errorLabel.stringValue = message
            errorBanner.isHidden = false
        }
    }

    @objc private func dismissError() {
        errorBanner.isHidden = true
    }

    public override func viewDidAppear() {
        super.viewDidAppear()
    }

    public func forceReload() {
        updateEmptyState()
    }

    public func showTrending() {
        isSearchMode = false
        trendingMode = true
        headerLabel.isHidden = false
        headerLabel.stringValue = "24h Trending"
        digestView.isHidden = true
        loadTrending()
    }

    private func loadTrending() {
        let seoyu = self.seoyu
        DispatchQueue.global(qos: .userInitiated).async {
            let topics = (try? seoyu.wikiTrending(limit: 40, offset: 0, category: nil)) ?? []
            DispatchQueue.main.async {
                guard self.trendingMode, !self.isSearchMode else { return }
                self.topics = topics
                self.tableView.reloadData()
                self.updateEmptyState()
            }
        }
    }

    private var lastReload: Date = .distantPast

    private func throttledReload() {
        guard trendingMode, !isSearchMode else { return }
        let now = Date()
        if now.timeIntervalSince(lastReload) < 0.5 { return }
        lastReload = now
        loadTrending()
    }

    private func handleProgress(_ note: Notification) {
        let total = (note.userInfo?["total"] as? UInt64) ?? 0
        guard topics.isEmpty, total > 0 else { return }
        let processed = (note.userInfo?["processed"] as? UInt64) ?? 0
        emptyLabel.stringValue = "Building wiki… \(processed)/\(total)"
        emptyLabel.isHidden = false
    }

    private func updateEmptyState() {
        if topics.isEmpty {
            emptyLabel.stringValue = isSearchMode ? "No results" : "Type to search wiki"
            emptyLabel.isHidden = false
        } else {
            emptyLabel.isHidden = true
        }
    }

    public func applySearch(query: String) {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if q.isEmpty {
            isSearchMode = false
            if trendingMode {
                showTrending()
            } else {
                headerLabel.isHidden = true
                digestView.isHidden = true
                self.topics = []
                self.tableView.reloadData()
                self.updateEmptyState()
            }
            return
        }
        isSearchMode = true
        headerLabel.isHidden = false
        headerLabel.stringValue = "Results for \"\(q)\""
        digestView.isHidden = true
        let seoyu = self.seoyu
        DispatchQueue.global(qos: .userInitiated).async {
            let results = (try? seoyu.wikiSearch(query: q, limit: 50)) ?? []
            DispatchQueue.main.async {
                self.topics = results
                self.tableView.reloadData()
                self.updateEmptyState()
            }
        }
    }

    @objc private func onRowClicked() {
        let row = tableView.clickedRow
        guard row >= 0, row < topics.count else { return }
        onTopicSelected?(topics[row])
    }

    public func numberOfRows(in tableView: NSTableView) -> Int { topics.count }

    public func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let topic = topics[row]
        let cell = NSTableCellView()
        cell.identifier = .init("topicCell")

        let title = NSTextField(labelWithString: titleForCurrentLanguage(topic))
        title.font = .systemFont(ofSize: 14, weight: .semibold)
        title.textColor = .labelColor
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false

        let msgs = NSTextField(labelWithString: "\(topic.messageCount) msgs")
        msgs.font = .systemFont(ofSize: 11, weight: .medium)
        msgs.textColor = .secondaryLabelColor
        msgs.translatesAutoresizingMaskIntoConstraints = false

        var metaItems: [NSView] = [msgs]
        if topic.trendingScore >= 1.0 {
            let fire = NSTextField(labelWithString: String(format: "🔥 %.1f", topic.trendingScore))
            fire.font = .systemFont(ofSize: 11, weight: .semibold)
            fire.textColor = .systemOrange
            metaItems.append(fire)
        }
        metaItems.append(NSView())
        let meta = NSStackView(views: metaItems)
        meta.orientation = .horizontal
        meta.spacing = 8
        meta.alignment = .centerY
        meta.translatesAutoresizingMaskIntoConstraints = false

        let vStack = NSStackView(views: [title, meta])
        vStack.orientation = .vertical
        vStack.alignment = .leading
        vStack.spacing = 6
        vStack.translatesAutoresizingMaskIntoConstraints = false

        cell.addSubview(vStack)
        NSLayoutConstraint.activate([
            vStack.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 14),
            vStack.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -14),
            vStack.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }


    private func titleForCurrentLanguage(_ topic: WikiTopicSummary) -> String {
        switch WikiLocale.current {
        case .ko:
            if let ko = topic.titleKo, !ko.isEmpty { return ko }
            return topic.title
        case .en:
            return topic.title
        }
    }
}
