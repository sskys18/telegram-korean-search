import Cocoa
import TGUIKit
import Seoyu

/// Placeholder scaffold for the Wiki tab. Phase 3 replaces this with the
/// real list view and viewer.
public final class WikiTabController: ViewController {
    private let seoyu: Seoyu

    public init(seoyu: Seoyu) {
        self.seoyu = seoyu
        super.init()
    }

    public override func viewDidLoad() {
        super.viewDidLoad()

        let placeholder = NSTextField(labelWithString: "Wiki — coming online")
        placeholder.alignment = .center
        placeholder.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(placeholder)
        NSLayoutConstraint.activate([
            placeholder.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            placeholder.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }
}
