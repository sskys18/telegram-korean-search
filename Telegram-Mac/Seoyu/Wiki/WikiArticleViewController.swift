import Cocoa
import Seoyu

public final class WikiArticleViewController: NSViewController,
    NSTableViewDataSource, NSTableViewDelegate
{
    private let seoyu: Seoyu
    private let topicId: Int64
    private let openChat: (Int64, Int64) -> Void

    private let titleLabel = NSTextField(labelWithString: "")
    private let articleText = NSTextView()
    private let sourcesTable = NSTableView()
    private var detail: WikiTopicDetail?
    private var sources: [SearchHit] = []

    public init(
        seoyu: Seoyu,
        topicId: Int64,
        openChat: @escaping (Int64, Int64) -> Void
    ) {
        self.seoyu = seoyu
        self.topicId = topicId
        self.openChat = openChat
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    public override func loadView() {
        titleLabel.font = .systemFont(ofSize: 18, weight: .bold)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.lineBreakMode = .byTruncatingTail

        articleText.isEditable = false
        articleText.isSelectable = true
        articleText.drawsBackground = false
        articleText.textContainerInset = NSSize(width: 0, height: 8)
        articleText.translatesAutoresizingMaskIntoConstraints = false

        let articleScroll = NSScrollView()
        articleScroll.documentView = articleText
        articleScroll.hasVerticalScroller = true
        articleScroll.translatesAutoresizingMaskIntoConstraints = false

        let column = NSTableColumn(identifier: .init("source"))
        column.width = 400
        sourcesTable.addTableColumn(column)
        sourcesTable.headerView = nil
        sourcesTable.rowHeight = 50
        sourcesTable.dataSource = self
        sourcesTable.delegate = self
        sourcesTable.target = self
        sourcesTable.action = #selector(onSourceClicked)

        let sourcesScroll = NSScrollView()
        sourcesScroll.documentView = sourcesTable
        sourcesScroll.hasVerticalScroller = true
        sourcesScroll.translatesAutoresizingMaskIntoConstraints = false

        let split = NSSplitView()
        split.isVertical = false
        split.dividerStyle = .thin
        split.translatesAutoresizingMaskIntoConstraints = false
        split.addArrangedSubview(articleScroll)
        split.addArrangedSubview(sourcesScroll)

        let root = NSStackView(views: [titleLabel, split])
        root.orientation = .vertical
        root.alignment = .leading
        root.spacing = 8
        root.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 0, right: 12)
        root.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.addSubview(root)
        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            root.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            root.topAnchor.constraint(equalTo: container.topAnchor),
            root.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            split.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            split.trailingAnchor.constraint(equalTo: root.trailingAnchor),
        ])
        self.view = container

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiLanguageChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.renderArticle() }
    }

    public override func viewDidAppear() {
        super.viewDidAppear()
        reload()
    }

    public func forceReload() {
        reload()
    }

    private func reload() {
        let seoyu = self.seoyu
        let topicId = self.topicId
        DispatchQueue.global(qos: .userInitiated).async {
            let detail = try? seoyu.wikiTopicDetail(topicId: topicId)
            let sources = (try? seoyu.wikiTopicMessages(topicId: topicId, limit: 50)) ?? []
            DispatchQueue.main.async {
                self.detail = detail ?? nil
                self.sources = sources
                self.renderArticle()
                self.sourcesTable.reloadData()
            }
        }
    }

    private func renderArticle() {
        guard let detail = detail else {
            titleLabel.stringValue = ""
            articleText.textStorage?.setAttributedString(NSAttributedString(string: ""))
            return
        }
        titleLabel.stringValue = titleForCurrentLanguage(detail.summary)
        let md: String
        switch WikiLocale.current {
        case .ko:
            md = detail.articleMdKo ?? detail.articleMd ?? ""
        case .en:
            md = detail.articleMd ?? ""
        }
        let rendered = MarkdownRenderer.render(md)
        articleText.textStorage?.setAttributedString(rendered)
    }

    private func titleForCurrentLanguage(_ summary: WikiTopicSummary) -> String {
        switch WikiLocale.current {
        case .ko:
            if let ko = summary.titleKo, !ko.isEmpty { return ko }
            return summary.title
        case .en:
            return summary.title
        }
    }

    @objc private func onSourceClicked() {
        let row = sourcesTable.clickedRow
        guard row >= 0, row < sources.count else { return }
        let hit = sources[row]
        openChat(hit.chatId, hit.messageId)
    }

    public func numberOfRows(in tableView: NSTableView) -> Int { sources.count }

    public func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let cell = WikiSourceCellView(frame: .zero)
        cell.identifier = .init("sourceCell")
        cell.configure(with: sources[row])
        return cell
    }
}
