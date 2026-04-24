import Foundation
import Postbox
import SwiftSignalKit
import Seoyu

/// Process-wide handle to the Rust sidecar. Opened once on app launch.
/// All calls into `Seoyu` go through this singleton so initialization
/// order and database path stay in one place.
public final class SeoyuBridge {
    public static let shared = SeoyuBridge()

    public private(set) var seoyu: Seoyu?
    public private(set) var initializationError: Error?
    private var ingestDisposable: Disposable?
    private let wikiObserverBridge = WikiObserverBridge()

    private init() {}

    /// Open (or create) the sqlite store under Application Support and
    /// run pending migrations. Safe to call multiple times; subsequent
    /// calls are no-ops.
    public func bootstrap() {
        guard seoyu == nil, initializationError == nil else { return }
        do {
            let fm = FileManager.default
            let base = try fm.url(
                for: .applicationSupportDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: true
            ).appendingPathComponent("telegram-korean-search", isDirectory: true)
            try fm.createDirectory(at: base, withIntermediateDirectories: true)
            let dbPath = base.appendingPathComponent("tg-korean-search.db").path
            let instance = try Seoyu(dbPath: dbPath)
            let version = instance.version()
            NSLog("[seoyu] opened store at %@, sidecar version %@", dbPath, version)
            self.seoyu = instance
        } catch {
            NSLog("[seoyu] bootstrap failed: %@", String(describing: error))
            self.initializationError = error
        }
    }

    /// Install the global Postbox observer that mirrors every stored or
    /// updated message into Seoyu. Safe to call multiple times per
    /// postbox; subsequent calls replace the previous observer so the
    /// sidecar only ever sees a single ingest stream.
    public func attach(postbox: Postbox) {
        guard let seoyu else { return }
        self.ingestDisposable?.dispose()
        let observer = SeoyuIngestObserver(seoyu: seoyu)
        self.ingestDisposable = postbox.installGlobalStoreOrUpdateMessageAction(action: observer)

        do {
            try seoyu.startWikiWorker()
            NSLog("[seoyu] wiki worker started")
            seoyu.setWikiObserver(observer: self.wikiObserverBridge)
            NSLog("[seoyu] wiki observer attached")
            seoyu.wikiRunPendingNow()
        } catch {
            NSLog("[seoyu] wiki worker start failed: %@", String(describing: error))
        }
    }

    deinit {
        self.ingestDisposable?.dispose()
        self.seoyu?.setWikiObserver(observer: nil)
        self.seoyu?.stopWikiWorker()
    }

    /// Run a global Korean-aware search. Returns an empty list if the
    /// sidecar is not initialized or the call fails so callers can
    /// treat this as a best-effort augmentation of native search.
    public func search(query: String, limit: UInt32 = 50) -> [SearchHit] {
        guard let seoyu, !query.isEmpty else { return [] }
        do {
            let page = try seoyu.search(
                query: query,
                scope: .all,
                limit: limit,
                cursor: nil
            )
            return page.items
        } catch {
            NSLog("[seoyu] search failed for %@: %@", query, String(describing: error))
            return []
        }
    }
}
