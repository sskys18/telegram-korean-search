import Foundation
import Postbox
import TelegramCore
import Seoyu

/// Postbox observer that forwards every stored or updated message into
/// the Rust sidecar so the Korean-aware index stays in sync with the
/// authoritative message history.
public final class SeoyuIngestObserver: StoreOrUpdateMessageAction, DeleteMessagesAction {
    private let seoyu: Seoyu

    public init(seoyu: Seoyu) {
        self.seoyu = seoyu
    }

    public func addOrUpdate(messages: [StoreMessage], transaction: Transaction) {
        var batch: [IndexedMessage] = []
        var emptied: [MessageRef] = []
        batch.reserveCapacity(messages.count)
        for message in messages {
            guard case let .Id(messageId) = message.id else { continue }
            guard messageId.namespace == Namespaces.Message.Cloud else { continue }
            let text = message.text
            if text.isEmpty {
                emptied.append(MessageRef(
                    chatId: messageId.peerId.toInt64(),
                    messageId: Int64(messageId.id)
                ))
                continue
            }
            batch.append(IndexedMessage(
                chatId: messageId.peerId.toInt64(),
                messageId: Int64(messageId.id),
                timestamp: Int64(message.timestamp),
                text: text,
                link: nil,
                senderId: message.authorId?.toInt64() ?? 0
            ))
        }
        if !batch.isEmpty {
            do {
                _ = try seoyu.indexMessages(messages: batch)
            } catch {
                NSLog("[seoyu] index failed for %d messages: %@", batch.count, String(describing: error))
            }
        }
        if !emptied.isEmpty {
            do {
                _ = try seoyu.deleteMessages(refs: emptied)
            } catch {
                NSLog("[seoyu] delete-on-empty failed for %d refs: %@", emptied.count, String(describing: error))
            }
        }
    }

    public func deleted(ids: [MessageId], transaction: Transaction) {
        var refs: [MessageRef] = []
        refs.reserveCapacity(ids.count)
        for id in ids {
            guard id.namespace == Namespaces.Message.Cloud else { continue }
            refs.append(MessageRef(
                chatId: id.peerId.toInt64(),
                messageId: Int64(id.id)
            ))
        }
        guard !refs.isEmpty else { return }
        do {
            _ = try seoyu.deleteMessages(refs: refs)
        } catch {
            NSLog("[seoyu] delete failed for %d refs: %@", refs.count, String(describing: error))
        }
    }
}
