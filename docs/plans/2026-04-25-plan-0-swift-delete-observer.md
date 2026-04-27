# Plan 0 — Swift Postbox delete observer (v8 leftover)

> Date: 2026-04-25
> Spec: `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md` §4.4
> Scope: finish the v8 message-index rewrite by wiring Postbox deletion
> into the sidecar's `delete_messages` UniFFI surface (already shipped).
> Size: 5 files touched, ~8 tasks. One-commit worth of work.

## Context

Phases 1–4 of the v8 plan shipped on 2026-04-24 (codex run, 46 edits).
The Rust sidecar exposes `Seoyu::delete_messages(refs: Vec<MessageRef>)`
and the IPC layer mirrors it (`sidecar/src/ipc/handlers.rs`,
`sidecar/src/uniffi_api.rs`). The Swift shell **does not yet call this
on real chat deletions** — when a user deletes a message in Telegram,
it disappears from the chat UI but the Seoyu index keeps the row.
Search keeps returning a hit for a message that no longer exists.

The upstream fork exposes `StoreOrUpdateMessageAction` in
`submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift:18-20`:

```swift
public protocol StoreOrUpdateMessageAction: AnyObject {
    func addOrUpdate(messages: [StoreMessage], transaction: Transaction)
}
```

There is no public deletion observer. All three deletion paths
(`deleteMessages`, `deleteMessagesInRange`, `clearHistory`) funnel into
`messageHistoryTable.removeMessages`, which appends
`MessageHistoryOperation.Remove([(MessageIndex, MessageTags)])` entries
to `currentOperationsByPeerId` (enum at
`submodules/telegram-ios/submodules/Postbox/Sources/MessageHistoryOperation.swift:3-10`).

`beforeCommit` (line 2433) is the one place that sees every operation
for a transaction right before it publishes. That is the cleanest hook
point: fire a delete observer there, reusing the same bag pattern as
`installedGlobalStoreOrUpdateMessageActions`.

## File map

| File | Change |
|---|---|
| `submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift` | Add `DeleteMessagesAction` protocol + global install + fire in `beforeCommit`. |
| `Telegram-Mac/Seoyu/SeoyuIngestObserver.swift` | Implement new protocol; call `seoyu.deleteMessages(refs:)`. |
| `Telegram-Mac/Seoyu/SeoyuBridge.swift` | Install the delete observer on `attach(postbox:)`. |
| `sidecar/tests/uniffi_surface.rs` | (optional sanity) Extend existing delete-propagation test — already added by codex but confirm signature match. |

Submodule note: the Postbox change lives in the vendored submodule at
`submodules/telegram-ios`. The submodule is already dirty (`m` in
`git status`). We land the change as an additional commit in the
submodule, then bump the submodule pointer in the main repo.

## Tasks

### Task 0.1 — Read the current Postbox install + fire sites

**Action**: familiarize with the exact surrounding code.

```bash
sed -n '18,20p' submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift
sed -n '1770,1780p' submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift
sed -n '4080,4100p' submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift
sed -n '2487,2600p' submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift
```

Expected: see the protocol at line 18, the bag declarations at
1772-1773, `installGlobalStoreOrUpdateMessageAction` at 4083, and
`beforeCommit` context. No edits.

### Task 0.2 — Add `DeleteMessagesAction` protocol + bag field

**File**: `submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift`

**Edit 1** — right after the existing protocol at line 18-20, insert:

```swift
public protocol DeleteMessagesAction: AnyObject {
    func deleted(ids: [MessageId], transaction: Transaction)
}
```

The edit's `old_string`:

```
public protocol StoreOrUpdateMessageAction: AnyObject {
    func addOrUpdate(messages: [StoreMessage], transaction: Transaction)
}
```

Replace with:

```
public protocol StoreOrUpdateMessageAction: AnyObject {
    func addOrUpdate(messages: [StoreMessage], transaction: Transaction)
}

public protocol DeleteMessagesAction: AnyObject {
    func deleted(ids: [MessageId], transaction: Transaction)
}
```

**Edit 2** — add bag field next to the existing
`installedGlobalStoreOrUpdateMessageActions` at line 1773.

`old_string`:

```
    var installedGlobalStoreOrUpdateMessageActions: Bag<StoreOrUpdateMessageAction> = Bag()
```

`new_string`:

```
    var installedGlobalStoreOrUpdateMessageActions: Bag<StoreOrUpdateMessageAction> = Bag()
    var installedGlobalDeleteMessagesActions: Bag<DeleteMessagesAction> = Bag()
```

### Task 0.3 — Add public install helper on `PostboxImpl`

**File**: same. Insert a new method immediately after
`installGlobalStoreOrUpdateMessageAction` at line 4083-4094.

`old_string`:

```
    public func installGlobalStoreOrUpdateMessageAction(action: StoreOrUpdateMessageAction) -> Disposable {
        let disposable = MetaDisposable()
        self.queue.async {
            let index = self.installedGlobalStoreOrUpdateMessageActions.add(action)
            disposable.set(ActionDisposable {
                self.queue.async {
                    self.installedGlobalStoreOrUpdateMessageActions.remove(index)
                }
            })
        }
        return disposable
    }
```

`new_string`:

```
    public func installGlobalStoreOrUpdateMessageAction(action: StoreOrUpdateMessageAction) -> Disposable {
        let disposable = MetaDisposable()
        self.queue.async {
            let index = self.installedGlobalStoreOrUpdateMessageActions.add(action)
            disposable.set(ActionDisposable {
                self.queue.async {
                    self.installedGlobalStoreOrUpdateMessageActions.remove(index)
                }
            })
        }
        return disposable
    }

    public func installGlobalDeleteMessagesAction(action: DeleteMessagesAction) -> Disposable {
        let disposable = MetaDisposable()
        self.queue.async {
            let index = self.installedGlobalDeleteMessagesActions.add(action)
            disposable.set(ActionDisposable {
                self.queue.async {
                    self.installedGlobalDeleteMessagesActions.remove(index)
                }
            })
        }
        return disposable
    }
```

### Task 0.4 — Mirror the public install helper on `Postbox` facade

**File**: same. At line 4997 there is a twin
`installGlobalStoreOrUpdateMessageAction` on the public `Postbox`
class that forwards to `impl`. Add a twin for delete.

`old_string`:

```
    public func installGlobalStoreOrUpdateMessageAction(action: StoreOrUpdateMessageAction) -> Disposable {
        let disposable = MetaDisposable()
        self.impl.with { impl in
            disposable.set(impl.installGlobalStoreOrUpdateMessageAction(action: action))
        }
        return disposable
    }
```

`new_string`:

```
    public func installGlobalStoreOrUpdateMessageAction(action: StoreOrUpdateMessageAction) -> Disposable {
        let disposable = MetaDisposable()
        self.impl.with { impl in
            disposable.set(impl.installGlobalStoreOrUpdateMessageAction(action: action))
        }
        return disposable
    }

    public func installGlobalDeleteMessagesAction(action: DeleteMessagesAction) -> Disposable {
        let disposable = MetaDisposable()
        self.impl.with { impl in
            disposable.set(impl.installGlobalDeleteMessagesAction(action: action))
        }
        return disposable
    }
```

### Task 0.5 — Fire observers in `beforeCommit`

**File**: same. Near the end of `beforeCommit`, just before
`PostboxTransaction` is constructed (line 2487), collect all
`.Remove` operations and fire observers exactly once per
transaction.

`old_string`:

```
        let updatedPeerTimeoutAttributes = self.peerTimeoutPropertiesTable.hasUpdates
        
        let transaction = PostboxTransaction(
```

`new_string`:

```
        let updatedPeerTimeoutAttributes = self.peerTimeoutPropertiesTable.hasUpdates

        if !self.installedGlobalDeleteMessagesActions.isEmpty {
            var deletedIds: [MessageId] = []
            for (_, operations) in self.currentOperationsByPeerId {
                for op in operations {
                    if case let .Remove(removals) = op {
                        for (index, _) in removals {
                            deletedIds.append(index.id)
                        }
                    }
                }
            }
            if !deletedIds.isEmpty {
                for action in self.installedGlobalDeleteMessagesActions.copyItems() {
                    action.deleted(ids: deletedIds, transaction: currentTransaction)
                }
            }
        }

        let transaction = PostboxTransaction(
```

**Note**: `Bag` has no `isEmpty`; check by `copyItems().isEmpty`
instead if the symbol is missing. Verify with:

```bash
grep -n "func isEmpty\|var isEmpty" submodules/telegram-ios/submodules/Postbox/Sources/Bag.swift
```

If `isEmpty` is not declared on `Bag`, replace
`!self.installedGlobalDeleteMessagesActions.isEmpty` with:

```swift
!self.installedGlobalDeleteMessagesActions.copyItems().isEmpty
```

### Task 0.6 — Implement the protocol in `SeoyuIngestObserver`

**File**: `Telegram-Mac/Seoyu/SeoyuIngestObserver.swift`

Current class implements `StoreOrUpdateMessageAction`. Add conformance
to `DeleteMessagesAction`.

`old_string`:

```swift
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
```

`new_string`:

```swift
public final class SeoyuIngestObserver: StoreOrUpdateMessageAction, DeleteMessagesAction {
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
```

**Verify UniFFI symbol names** — the UniFFI-generated `MessageRef`
might use `chat_id` / `message_id` (snake) or `chatId` / `messageId`
(camel) on the Swift side. Confirm with:

```bash
grep -n "public struct MessageRef\|MessageRef(" \
    packages/Seoyu/Sources/Seoyu/*.swift 2>/dev/null | head
grep -n "fn delete_messages\|deleteMessages" \
    packages/Seoyu/Sources/Seoyu/*.swift sidecar/src/uniffi_api.rs 2>/dev/null | head
```

If the generated API uses `chat_id`, adjust the `MessageRef(...)` call
accordingly. Method name on Swift side is almost certainly
`deleteMessages(refs:)` — UniFFI camelCases Rust names.

### Task 0.7 — Install the delete observer from `SeoyuBridge.attach`

**File**: `Telegram-Mac/Seoyu/SeoyuBridge.swift`

Add a second disposable for the delete observer. The same
`SeoyuIngestObserver` instance implements both protocols, so install
it twice against the two different install APIs.

`old_string`:

```swift
    private var ingestDisposable: Disposable?
    private let wikiObserverBridge = WikiObserverBridge()

    private init() {}
```

`new_string`:

```swift
    private var ingestDisposable: Disposable?
    private var deleteDisposable: Disposable?
    private let wikiObserverBridge = WikiObserverBridge()

    private init() {}
```

`old_string`:

```swift
    public func attach(postbox: Postbox) {
        guard let seoyu else { return }
        self.ingestDisposable?.dispose()
        let observer = SeoyuIngestObserver(seoyu: seoyu)
        self.ingestDisposable = postbox.installGlobalStoreOrUpdateMessageAction(action: observer)

        do {
```

`new_string`:

```swift
    public func attach(postbox: Postbox) {
        guard let seoyu else { return }
        self.ingestDisposable?.dispose()
        self.deleteDisposable?.dispose()
        let observer = SeoyuIngestObserver(seoyu: seoyu)
        self.ingestDisposable = postbox.installGlobalStoreOrUpdateMessageAction(action: observer)
        self.deleteDisposable = postbox.installGlobalDeleteMessagesAction(action: observer)

        do {
```

`old_string`:

```swift
    deinit {
        self.ingestDisposable?.dispose()
        self.seoyu?.setWikiObserver(observer: nil)
        self.seoyu?.stopWikiWorker()
    }
```

`new_string`:

```swift
    deinit {
        self.ingestDisposable?.dispose()
        self.deleteDisposable?.dispose()
        self.seoyu?.setWikiObserver(observer: nil)
        self.seoyu?.stopWikiWorker()
    }
```

### Task 0.8 — Build + manual verification

Command:

```bash
./scripts/build-dev.sh --run
```

Expected: app launches, `[seoyu] opened store …`, `[seoyu] wiki worker
started`, `[seoyu] wiki observer attached` in Console.

Manual verify:

1. Send a distinctive test message in any cloud chat (e.g. `SEOYU-DELETE-TEST-20260425`).
2. Wait a beat; confirm the search bar finds it via Seoyu hit (search for `SEOYU-DELETE`).
3. Long-press the message in Telegram-Mac → Delete for me.
4. Re-run the search. **Expected**: zero Seoyu hits for
   `SEOYU-DELETE-TEST-20260425`. If the hit persists, the observer did
   not fire — check Console for `[seoyu] delete failed` or absence of
   any delete trace.

Additional smoke: pick a chat and delete ≥5 messages in one action
(multi-select). All five should drop out of search within one tick.

SQL-level verify (optional):

```bash
sqlite3 ~/Library/Application\ Support/telegram-korean-search/tg-korean-search.db \
  "SELECT COUNT(*) FROM messages WHERE text_plain LIKE '%SEOYU-DELETE-TEST-20260425%';"
```

Expected output: `0` after delete.

### Task 0.9 — Commit

Two commits — one in the submodule, one in main repo.

In submodule:

```bash
cd submodules/telegram-ios
git add submodules/Postbox/Sources/Postbox.swift
git commit -m "$(cat <<'EOF'
feat(postbox): expose global delete-messages observer

Adds DeleteMessagesAction protocol + installGlobalDeleteMessagesAction
helper. Fires once per transaction in beforeCommit by scanning
currentOperationsByPeerId for .Remove entries. Parity with the existing
StoreOrUpdateMessageAction hook used by Seoyu.
EOF
)"
cd -
```

In main repo:

```bash
git add submodules/telegram-ios \
    Telegram-Mac/Seoyu/SeoyuIngestObserver.swift \
    Telegram-Mac/Seoyu/SeoyuBridge.swift
git commit -m "$(cat <<'EOF'
feat(seoyu): wire Postbox deletions into sidecar delete_messages

Picks up the new DeleteMessagesAction hook in the telegram-ios fork
and forwards deleted cloud-message ids to Seoyu::delete_messages, so
search stops returning hits for messages the user has deleted.

Completes phase 2b of the v8 message-index rewrite.
EOF
)"
```

## Self-review checklist (author before handoff)

- [ ] Every section in §4.4 of the spec is covered.
- [ ] No placeholders; every step has exact code/paths/commands.
- [ ] `MessageRef` constructor style verified against UniFFI output.
- [ ] Verification in Task 0.8 hits the actual DB, not just the UI.
- [ ] Submodule + main-repo commits are separate.
- [ ] `isEmpty` fallback noted in Task 0.5 if `Bag` lacks the accessor.

## Known non-blockers (do NOT fix in this plan)

- Edit propagation (text-change reclassify): separate concern, handled
  by Wiki v2 `text_hash` in §6.1.
- `deleteMessagesInRange` / `clearHistory` — already covered because
  both paths funnel through `.Remove` ops in `currentOperationsByPeerId`.
- Non-cloud namespaces (secret chats, local drafts) — explicitly
  skipped in `deleted(ids:transaction:)` by the namespace guard,
  matching the existing addOrUpdate behavior.
