import Cocoa
import TGUIKit
import Seoyu

public final class WikiTabController: ViewController {
    private let seoyu: Seoyu
    public var openChat: ((Int64, Int64) -> Void)?

    private let toolbar = NSStackView()
    private let langButton = NSButton()
    private let searchButton = NSButton()
    private let runButton = NSButton()
    private var pendingCount: UInt64 = 0

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

        containerView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(containerView)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 8),
            toolbar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8),
            toolbar.topAnchor.constraint(equalTo: view.topAnchor, constant: 6),
            toolbar.heightAnchor.constraint(equalToConstant: 24),
            containerView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            containerView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            containerView.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 6),
            containerView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        push(listController, animated: false)

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiProgress,
            object: nil,
            queue: .main
        ) { [weak self] note in
            self?.pendingCount = (note.userInfo?["pending"] as? UInt64) ?? 0
            self?.updateRunButton()
        }
        NotificationCenter.default.addObserver(
            forName: .seoyuWikiLanguageChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.updateLangButton() }
    }

    private func setupToolbar() {
        toolbar.orientation = .horizontal
        toolbar.spacing = 8
        toolbar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(toolbar)

        langButton.bezelStyle = .inline
        langButton.target = self
        langButton.action = #selector(toggleLanguage)
        updateLangButton()

        searchButton.bezelStyle = .inline
        searchButton.title = "Search"
        searchButton.target = self
        searchButton.action = #selector(presentSearch)

        runButton.bezelStyle = .inline
        runButton.title = "Run classify"
        runButton.target = self
        runButton.action = #selector(runPending)
        updateRunButton()

        toolbar.addArrangedSubview(langButton)
        toolbar.addArrangedSubview(searchButton)
        toolbar.addArrangedSubview(NSView())
        toolbar.addArrangedSubview(runButton)
    }

    private func updateLangButton() {
        langButton.title = WikiLocale.current == .en ? "EN" : "KO"
    }

    private func updateRunButton() {
        runButton.isEnabled = pendingCount > 0
    }

    @objc private func toggleLanguage() {
        WikiLocale.current = (WikiLocale.current == .en) ? .ko : .en
        updateLangButton()
    }

    @objc private func runPending() {
        seoyu.wikiRunPendingNow()
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
        addChild(child)
        child.view.translatesAutoresizingMaskIntoConstraints = false
        containerView.addSubview(child.view)
        NSLayoutConstraint.activate([
            child.view.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
            child.view.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
            child.view.topAnchor.constraint(equalTo: containerView.topAnchor),
            child.view.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
        ])
        pageStack.append(child)
    }

    @discardableResult
    public func popToRoot() -> Bool {
        guard pageStack.count > 1 else { return false }
        while pageStack.count > 1 {
            let top = pageStack.removeLast()
            top.view.removeFromSuperview()
            top.removeFromParent()
        }
        if let root = pageStack.last {
            root.view.translatesAutoresizingMaskIntoConstraints = false
            containerView.addSubview(root.view)
            NSLayoutConstraint.activate([
                root.view.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
                root.view.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
                root.view.topAnchor.constraint(equalTo: containerView.topAnchor),
                root.view.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
            ])
        }
        return true
    }
}
