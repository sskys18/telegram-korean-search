import Foundation
import Seoyu

/// Process-wide handle to the Rust sidecar. Opened once on app launch.
/// All calls into `Seoyu` go through this singleton so initialization
/// order and database path stay in one place.
public final class SeoyuBridge {
    public static let shared = SeoyuBridge()

    public private(set) var seoyu: Seoyu?
    public private(set) var initializationError: Error?

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
