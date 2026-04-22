import Foundation
import Postbox
import TelegramCore
import Seoyu

/// Postbox observer that forwards every stored or updated message into
/// the Rust sidecar so the Korean-aware index stays in sync with the
/// authoritative message history.
public final class SeoyuIngestObserver: StoreOrUpdateMessageAction {
    private let seoyu: Seoyu

    public init(seoyu: Seoyu) {
        self.seoyu = seoyu
    }

    public func addOrUpdate(messages: [StoreMessage], transaction: Transaction) {
        var batch: [IndexedMessage] = []
        batch.reserveCapacity(messages.count)
        for message in messages {
            guard case let .Id(messageId) = message.id else { continue }
            guard messageId.namespace == Namespaces.Message.Cloud else { continue }
            let text = message.text
            guard !text.isEmpty else { continue }
            batch.append(IndexedMessage(
                chatId: messageId.peerId.toInt64(),
                messageId: Int64(messageId.id),
                timestamp: Int64(message.timestamp),
                text: text,
                link: nil
            ))
        }
        guard !batch.isEmpty else { return }
        do {
            _ = try seoyu.indexMessages(messages: batch)
        } catch {
            NSLog("[seoyu] index failed for %d messages: %@", batch.count, String(describing: error))
        }
    }
}
