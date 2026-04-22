# Seoyu

Swift Package that re-exports the Rust sidecar (Korean search + wiki
core) via UniFFI. The Telegram-Mac app target imports this package and
calls the `Seoyu` object directly — there is no socket, no subprocess,
no IPC.

## Layout

```
packages/Seoyu/
  Package.swift                       public manifest
  Sources/
    Seoyu/
      Generated/seoyu.swift           (generated, gitignored)
    SeoyuFFI/
      include/seoyuFFI.h              (generated, gitignored)
      include/module.modulemap        (generated, gitignored)
  SeoyuFFI.xcframework/               (generated, gitignored)
```

## How to (re)generate

From the repo root:

```
./scripts/build-seoyu-xcframework.sh
```

The script:
1. Builds the Rust static library for both macOS architectures.
2. lipos them into a universal binary.
3. Runs UniFFI's bindgen on the dylib to emit the Swift wrapper,
   C header, and modulemap.
4. Assembles the `.xcframework` that this package's `Package.swift`
   declares as a binary target.

A fresh clone won't have any of the generated files yet — run the
script once and Xcode will pick them up automatically.

## Usage from Swift

```swift
import Seoyu

let seoyu = try Seoyu(dbPath: "~/Library/Application Support/telegram-seoyu/seoyu.db")

try seoyu.upsertChat(chat: ChatInfo(
    chatId: 42,
    title: "Crypto News",
    chatType: "channel",
    username: "crypto_ko",
    accessHash: nil,
    isExcluded: false
))

try seoyu.indexMessages(messages: [
    IndexedMessage(chatId: 42, messageId: 100, timestamp: 1_700_000_000,
                   text: "삼성전자 실적 발표", link: nil)
])

let page = try seoyu.search(
    query: "삼성",
    scope: .all,
    limit: 30,
    cursor: nil
)
for hit in page.items {
    print(hit.messageId, hit.text)
}
```

## Where the logic lives

None of the interesting code lives in this package. Everything — the
Hangul normalizer, the FTS5 query planner, the wiki pipeline, the
schema migrations — lives in the Rust crate at `../sidecar/`. This
package is only the cross-language bridge.
