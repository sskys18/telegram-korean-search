import Cocoa
import TGUIKit
import Seoyu

public final class WikiTabController: ViewController {
    private let seoyu: Seoyu
    public var openChat: ((Int64, Int64) -> Void)?

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

        containerView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(containerView)
        NSLayoutConstraint.activate([
            containerView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            containerView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            containerView.topAnchor.constraint(equalTo: view.topAnchor),
            containerView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        push(listController, animated: false)
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
