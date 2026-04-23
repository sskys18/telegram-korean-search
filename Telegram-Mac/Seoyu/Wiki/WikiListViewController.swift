import Cocoa
import Seoyu

public final class WikiListViewController: NSViewController,
    NSTableViewDataSource, NSTableViewDelegate
{
    private let seoyu: Seoyu
    private let tableView = NSTableView()
    private let digestView = WikiDigestCardView()
    private let chipsView = WikiCategoryChipsView()
    private let emptyLabel = NSTextField(labelWithString: "")

    private var topics: [WikiTopicSummary] = []
    private var seedTopics: [WikiTopicSummary]?
    private var selectedCategory: String? = nil

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

        root.addArrangedSubview(digestView)
        root.addArrangedSubview(chipsView)

        let column = NSTableColumn(identifier: .init("topic"))
        column.width = 400
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 44
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.action = #selector(onRowClicked)

        let scroll = NSScrollView()
        scroll.documentView = tableView
        scroll.hasVerticalScroller = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        root.addArrangedSubview(scroll)

        emptyLabel.alignment = .center
        emptyLabel.textColor = .secondaryLabelColor
        emptyLabel.isHidden = true
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.addSubview(root)
        container.addSubview(emptyLabel)
        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            root.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            root.topAnchor.constraint(equalTo: container.topAnchor),
            root.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            emptyLabel.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: container.centerYAnchor),
        ])
        self.view = container

        chipsView.onCategorySelected = { [weak self] name in
            self?.selectedCategory = name
            self?.reload()
        }

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
    }

    public override func viewDidAppear() {
        super.viewDidAppear()
        reload()
    }

    private var lastReload: Date = .distantPast

    private func throttledReload() {
        let now = Date()
        if now.timeIntervalSince(lastReload) < 0.5 {
            return
        }
        lastReload = now
        reload()
    }

    private func reload() {
        if let seed = seedTopics {
            self.topics = seed
            self.tableView.reloadData()
            self.updateEmptyState()
            return
        }
        let seoyu = self.seoyu
        let cat = self.selectedCategory
        DispatchQueue.global(qos: .userInitiated).async {
            let topics = (try? seoyu.wikiTrending(limit: 40, offset: 0, category: cat)) ?? []
            let digest = try? seoyu.wikiDigestToday()
            let cats = (try? seoyu.wikiCategories()) ?? []
            DispatchQueue.main.async {
                self.topics = topics
                self.digestView.configure(with: digest)
                self.chipsView.configure(with: cats, selected: cat)
                self.tableView.reloadData()
                self.updateEmptyState()
            }
        }
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
            emptyLabel.stringValue = "No topics yet"
            emptyLabel.isHidden = false
        } else {
            emptyLabel.isHidden = true
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
        title.translatesAutoresizingMaskIntoConstraints = false
        title.lineBreakMode = .byTruncatingTail
        let count = NSTextField(labelWithString: "\(topic.messageCount) msgs")
        count.textColor = .secondaryLabelColor
        count.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView(views: [title, NSView(), count])
        stack.orientation = .horizontal
        stack.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -12),
            stack.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
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
