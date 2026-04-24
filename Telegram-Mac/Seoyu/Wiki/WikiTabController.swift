import Cocoa
import TGUIKit
import Seoyu

public final class WikiTabController: ViewController {
    private let seoyu: Seoyu
    public var openChat: ((Int64, Int64) -> Void)?

    private let backButton = NSButton()
    private let langButton = NSButton()
    private let searchButton = NSButton()
    private let statusLabel = NSTextField(labelWithString: "")
    private var pendingCount: UInt64 = 0
    private var processedCount: UInt64 = 0
    private var totalCount: UInt64 = 0

    private let containerView = NSView()
    private lazy var listController: WikiListViewController = {
        let lc = WikiListViewController(seoyu: seoyu)
        lc.onTopicSelected = { [weak self] topic in
            self?.pushArticle(topicId: topic.id)
        }
        return lc
    }()
    private var pageStack: [NSViewController] = []

    public init(seoyu: Seoyu, openChat: ((Int64, Int64) -> Void)? = nil) {
        self.seoyu = seoyu
        self.openChat = openChat
        super.init()
    }

    public override func viewDidLoad() {
        super.viewDidLoad()

        setupToolbar()

        containerView.translatesAutoresizingMaskIntoConstraints = true
        view.addSubview(containerView)

        layoutManual()

        push(listController, animated: false)
        layoutManual()

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiProgress,
            object: nil,
            queue: .main
        ) { [weak self] note in
            guard let self else { return }
            self.pendingCount = (note.userInfo?["pending"] as? UInt64) ?? 0
            self.processedCount = (note.userInfo?["processed"] as? UInt64) ?? 0
            self.totalCount = (note.userInfo?["total"] as? UInt64) ?? 0
            self.updateStatusLabel()
        }
        NotificationCenter.default.addObserver(
            forName: .seoyuWikiLanguageChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.updateLangButton() }
    }

    public override func viewDidResized(_ size: NSSize) {
        super.viewDidResized(size)
        layoutManual()
    }

    private func layoutManual() {
        let size = view.frame.size
        let w = size.width
        let h = size.height
        let toolbarH: CGFloat = 24
        let topPad: CGFloat = 6
        let spacing: CGFloat = 6
        let btnGap: CGFloat = 8
        let sidePad: CGFloat = 8
        backButton.sizeToFit()
        langButton.sizeToFit()
        searchButton.sizeToFit()
        statusLabel.sizeToFit()
        let onArticle = pageStack.count > 1
        backButton.isHidden = !onArticle
        langButton.isHidden = onArticle
        searchButton.isHidden = onArticle
        var x = sidePad
        if onArticle {
            backButton.frame = NSRect(x: x, y: topPad, width: max(backButton.frame.width, 24), height: toolbarH)
            x += backButton.frame.width + btnGap
        } else {
            langButton.frame = NSRect(x: x, y: topPad, width: langButton.frame.width, height: toolbarH)
            x += langButton.frame.width + btnGap
            searchButton.frame = NSRect(x: x, y: topPad, width: searchButton.frame.width, height: toolbarH)
        }
        let statusW = statusLabel.frame.width
        statusLabel.frame = NSRect(x: max(w - sidePad - statusW, x + btnGap), y: topPad + 4, width: statusW, height: toolbarH - 4)
        let containerY = topPad + toolbarH + spacing
        containerView.frame = NSRect(x: 0, y: containerY, width: w, height: max(h - containerY, 0))
        for child in pageStack {
            child.view.frame = containerView.bounds
            child.view.needsUpdateConstraints = true
            child.view.updateConstraintsForSubtreeIfNeeded()
            child.view.needsLayout = true
            child.view.layoutSubtreeIfNeeded()
        }
    }

    private func setupToolbar() {
        backButton.bezelStyle = .inline
        backButton.isBordered = false
        backButton.target = self
        backButton.action = #selector(popBack)
        backButton.isHidden = true
        let cfg = NSImage.SymbolConfiguration(pointSize: 13, weight: .semibold)
        backButton.image = NSImage(systemSymbolName: "chevron.backward", accessibilityDescription: "Back")?.withSymbolConfiguration(cfg)
        backButton.imagePosition = .imageLeading
        backButton.attributedTitle = NSAttributedString(string: " Back", attributes: [
            .foregroundColor: NSColor.controlAccentColor,
            .font: NSFont.systemFont(ofSize: 12, weight: .medium),
        ])

        langButton.bezelStyle = .inline
        langButton.target = self
        langButton.action = #selector(toggleLanguage)
        updateLangButton()

        searchButton.bezelStyle = .inline
        searchButton.title = "Search"
        searchButton.target = self
        searchButton.action = #selector(presentSearch)

        statusLabel.font = NSFont.systemFont(ofSize: 11)
        statusLabel.textColor = .secondaryLabelColor
        updateStatusLabel()

        view.addSubview(backButton)
        view.addSubview(langButton)
        view.addSubview(searchButton)
        view.addSubview(statusLabel)
    }

    @objc private func popBack() {
        guard pageStack.count > 1 else { return }
        let top = pageStack.removeLast()
        top.view.removeFromSuperview()
        if let prev = pageStack.last {
            prev.view.translatesAutoresizingMaskIntoConstraints = true
            prev.view.autoresizingMask = [.width, .height]
            prev.view.frame = containerView.bounds
            containerView.addSubview(prev.view)
            if let list = prev as? WikiListViewController {
                list.forceReload()
            }
        }
        layoutManual()
    }

    private func updateLangButton() {
        langButton.title = WikiLocale.current == .en ? "EN" : "KO"
    }

    private func updateStatusLabel() {
        if pendingCount > 0 {
            statusLabel.stringValue = "queued \(pendingCount)"
        } else if totalCount > 0 {
            statusLabel.stringValue = "\(processedCount)/\(totalCount)"
        } else {
            statusLabel.stringValue = "idle"
        }
        statusLabel.sizeToFit()
        layoutManual()
    }

    @objc private func toggleLanguage() {
        WikiLocale.current = (WikiLocale.current == .en) ? .ko : .en
        updateLangButton()
    }

    @objc private func presentSearch() {
        let sheet = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 80),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        sheet.title = "Search wiki"
        let field = NSTextField(frame: NSRect(x: 12, y: 36, width: 296, height: 24))
        field.placeholderString = "Topic title"
        field.target = self
        field.action = #selector(submitSearch(_:))
        let cancel = NSButton(frame: NSRect(x: 232, y: 6, width: 76, height: 24))
        cancel.title = "Cancel"
        cancel.bezelStyle = .rounded
        cancel.target = self
        cancel.action = #selector(cancelSearch(_:))
        sheet.contentView?.addSubview(field)
        sheet.contentView?.addSubview(cancel)
        sheet.initialFirstResponder = field
        searchSheet = sheet
        view.window?.beginSheet(sheet, completionHandler: nil)
    }

    private weak var searchSheet: NSWindow?

    @objc private func cancelSearch(_ sender: NSButton) {
        if let sheet = searchSheet {
            view.window?.endSheet(sheet)
            searchSheet = nil
        }
    }

    @objc private func submitSearch(_ sender: NSTextField) {
        let query = sender.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if let sheet = searchSheet {
            view.window?.endSheet(sheet)
            searchSheet = nil
        }
        guard !query.isEmpty else { return }
        let seoyu = self.seoyu
        DispatchQueue.global(qos: .userInitiated).async {
            let results = (try? seoyu.wikiSearch(query: query, limit: 50)) ?? []
            DispatchQueue.main.async {
                let resultsVC = WikiListViewController(seoyu: seoyu, seed: results)
                resultsVC.onTopicSelected = { [weak self] topic in
                    self?.pushArticle(topicId: topic.id)
                }
                self.push(resultsVC, animated: true)
            }
        }
    }

    private func pushArticle(topicId: Int64) {
        let article = WikiArticleViewController(
            seoyu: seoyu,
            topicId: topicId,
            openChat: { [weak self] chatId, messageId in
                self?.openChat?(chatId, messageId)
            }
        )
        push(article, animated: true)
    }

    private func push(_ child: NSViewController, animated: Bool) {
        if let current = pageStack.last {
            current.view.removeFromSuperview()
        }
        child.view.translatesAutoresizingMaskIntoConstraints = true
        child.view.autoresizingMask = [.width, .height]
        child.view.frame = containerView.bounds
        containerView.addSubview(child.view)
        pageStack.append(child)
        // viewDidAppear is unreliable without proper VC parenting, so
        // poke the new page to load its data immediately.
        if let list = child as? WikiListViewController {
            list.forceReload()
        } else if let article = child as? WikiArticleViewController {
            article.forceReload()
        }
        layoutManual()
    }

    @discardableResult
    public func popToRoot() -> Bool {
        guard pageStack.count > 1 else { return false }
        while pageStack.count > 1 {
            let top = pageStack.removeLast()
            top.view.removeFromSuperview()
        }
        if let root = pageStack.last {
            root.view.translatesAutoresizingMaskIntoConstraints = true
            root.view.autoresizingMask = [.width, .height]
            root.view.frame = containerView.bounds
            containerView.addSubview(root.view)
        }
        return true
    }
}
