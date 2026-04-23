import Foundation
import Seoyu

/// Bridges the UniFFI `WikiObserver` callback trait to NotificationCenter
/// posts on the main queue. A single instance is held by SeoyuBridge
/// for the app's lifetime.
public final class WikiObserverBridge: WikiObserver {
    public init() {}

    public func onProgress(processed: UInt64, pending: UInt64, total: UInt64) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(
                name: .seoyuWikiProgress,
                object: nil,
                userInfo: [
                    "processed": processed,
                    "pending": pending,
                    "total": total,
                ]
            )
        }
    }

    public func onError(message: String, recoverable: Bool) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(
                name: .seoyuWikiError,
                object: nil,
                userInfo: [
                    "message": message,
                    "recoverable": recoverable,
                ]
            )
        }
    }

    public func onTopicsChanged() {
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .seoyuWikiTopicsChanged, object: nil)
        }
    }
}
