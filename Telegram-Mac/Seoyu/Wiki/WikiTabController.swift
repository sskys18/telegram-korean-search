import Cocoa
import TGUIKit
import Seoyu

public final class WikiTabController: ViewController, NSSearchFieldDelegate {
    private let seoyu: Seoyu
    public var openChat: ((Int64, Int64) -> Void)?

    private let backButton = NSButton()
    private let settingsButton = NSButton()
    private let searchField = NSSearchField()

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
        listController.showTrending()
    }

    public override func viewDidResized(_ size: NSSize) {
        super.viewDidResized(size)
        layoutManual()
        registerSearchResponder()
    }

    public override func firstResponder() -> NSResponder? {
        if pageStack.count <= 1 {
            return searchField
        }
        return nil
    }

    private var responderRegistered = false

    private func registerSearchResponder() {
        guard !responderRegistered, let win = view.window as? Window else { return }
        responderRegistered = true
        win.set(responder: { [weak self] () -> NSResponder? in
            guard let self else { return nil }
            if self.pageStack.count > 1 { return nil }
            guard let window = self.view.window else { return nil }
            let fr = window.firstResponder
            if fr === self.searchField { return self.searchField }
            if let view = fr as? NSView, view.isDescendant(of: self.view) {
                return self.searchField
            }
            if let editor = fr as? NSText, let host = editor.delegate as? NSView, host.isDescendant(of: self.view) {
                return self.searchField
            }
            return nil
        }, with: self, priority: .modal)
    }

    private func layoutManual() {
        let size = view.frame.size
        let w = size.width
        let h = size.height
        let toolbarH: CGFloat = 24
        let topPad: CGFloat = 6
        let spacing: CGFloat = 6
        let sidePad: CGFloat = 8
        backButton.sizeToFit()
        settingsButton.sizeToFit()
        let onArticle = pageStack.count > 1
        backButton.isHidden = !onArticle
        searchField.isHidden = onArticle
        let leftEdge: CGFloat
        if onArticle {
            backButton.frame = NSRect(x: sidePad, y: topPad, width: max(backButton.frame.width, 24), height: toolbarH)
            leftEdge = sidePad + backButton.frame.width + 8
        } else {
            leftEdge = sidePad
        }
        let gearW = max(settingsButton.frame.width, 24)
        let gearX = w - sidePad - gearW
        settingsButton.frame = NSRect(x: gearX, y: topPad, width: gearW, height: toolbarH)
        if !onArticle {
            let searchW = max(gearX - 8 - leftEdge, 0)
            searchField.frame = NSRect(x: leftEdge, y: topPad, width: searchW, height: toolbarH)
        }
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

        settingsButton.bezelStyle = .inline
        settingsButton.isBordered = false
        settingsButton.image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: "Settings")?.withSymbolConfiguration(cfg)
        settingsButton.imagePosition = .imageOnly
        settingsButton.target = self
        settingsButton.action = #selector(showSettingsMenu(_:))

        searchField.placeholderString = "Search wiki"
        searchField.target = self
        searchField.action = #selector(onSearchSubmit(_:))
        searchField.delegate = self
        searchField.sendsSearchStringImmediately = false
        searchField.sendsWholeSearchString = true
        searchField.controlSize = .small
        searchField.font = .systemFont(ofSize: 12)

        view.addSubview(backButton)
        view.addSubview(settingsButton)
        view.addSubview(searchField)
    }

    @objc private func onSearchSubmit(_ sender: NSSearchField) {
        if pageStack.last !== listController {
            popToRoot()
        }
        listController.applySearch(query: sender.stringValue)
    }

    public func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        if commandSelector == #selector(NSResponder.insertNewline(_:)) {
            onSearchSubmit(searchField)
            return true
        }
        return false
    }

    @objc private func showSettingsMenu(_ sender: NSButton) {
        let menu = NSMenu()
        let trending = NSMenuItem(title: "24h Trending", action: #selector(showTrending), keyEquivalent: "")
        trending.target = self
        menu.addItem(trending)
        menu.addItem(.separator())
        let langItem = NSMenuItem(title: "Language", action: nil, keyEquivalent: "")
        let langSub = NSMenu()
        let ko = NSMenuItem(title: "한국어", action: #selector(setLangKo), keyEquivalent: "")
        ko.target = self
        ko.state = WikiLocale.current == .ko ? .on : .off
        let en = NSMenuItem(title: "English", action: #selector(setLangEn), keyEquivalent: "")
        en.target = self
        en.state = WikiLocale.current == .en ? .on : .off
        langSub.addItem(ko)
        langSub.addItem(en)
        langItem.submenu = langSub
        menu.addItem(langItem)
        let p = NSPoint(x: 0, y: sender.bounds.height)
        menu.popUp(positioning: nil, at: p, in: sender)
    }

    @objc private func showTrending() {
        if pageStack.last !== listController {
            popToRoot()
        }
        listController.showTrending()
    }

    @objc private func setLangKo() { WikiLocale.current = .ko }
    @objc private func setLangEn() { WikiLocale.current = .en }

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
