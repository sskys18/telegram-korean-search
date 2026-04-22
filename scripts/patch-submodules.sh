#!/bin/bash
# Re-apply the local patches to submodules/telegram-ios that we carry
# out-of-tree. Run this once after every `git submodule update` that
# touches submodules/telegram-ios, otherwise Xcode builds will fail
# with deploy-target errors and Seoyu ingest will silently do nothing.
#
# Patches:
#   1. Bump every telegram-ios submodule Package.swift from
#      .macOS(.v10_13) (and related pre-12 values) to .macOS(.v12).
#      Xcode 26 otherwise trips the Swift driver's Metal-cryptex
#      back-deploy path even on arm64 macOS 13+.
#   2. Add a `Bag<StoreOrUpdateMessageAction>` broadcast observer to
#      Postbox that fires on every addMessages call regardless of
#      peerId, plus the public `installGlobalStoreOrUpdateMessageAction`
#      entry points. The upstream API is per-peer-id, which is
#      unusable for a global search/index mirror.
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- (1) Package.swift deploy-target bumps ----
MANIFESTS=$(grep -rl "\.v10_1[0-4]" submodules/telegram-ios/submodules/*/Package.swift 2>/dev/null || true)
if [[ -n "$MANIFESTS" ]]; then
  echo "Bumping deploy targets in:"
  echo "$MANIFESTS" | sed 's/^/  /'
  # shellcheck disable=SC2086
  sed -i '' 's/\.v10_1[0-4]/.v12/g' $MANIFESTS
fi

# ---- (2) Postbox global observer ----
POSTBOX=submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift
if ! grep -q "installedGlobalStoreOrUpdateMessageActions" "$POSTBOX"; then
  echo "Patching $POSTBOX"
  python3 - "$POSTBOX" <<'PY'
import sys, re
p = sys.argv[1]
src = open(p).read()

storage_needle = "var installedStoreOrUpdateMessageActionsByPeerId: [PeerId: Bag<StoreOrUpdateMessageAction>] = [:]"
storage_patch = storage_needle + "\n    var installedGlobalStoreOrUpdateMessageActions: Bag<StoreOrUpdateMessageAction> = Bag()"
assert storage_needle in src, "storage anchor missing"
src = src.replace(storage_needle, storage_patch, 1)

fire_needle = (
"            if let bag = self.installedStoreOrUpdateMessageActionsByPeerId[peerId] {\n"
"                for f in bag.copyItems() {\n"
"                    f.addOrUpdate(messages: peerMessages, transaction: transaction)\n"
"                }\n"
"            }\n"
"        }\n"
"        \n"
"        return addResult\n"
"    }"
)
fire_patch = (
"            if let bag = self.installedStoreOrUpdateMessageActionsByPeerId[peerId] {\n"
"                for f in bag.copyItems() {\n"
"                    f.addOrUpdate(messages: peerMessages, transaction: transaction)\n"
"                }\n"
"            }\n"
"            for f in self.installedGlobalStoreOrUpdateMessageActions.copyItems() {\n"
"                f.addOrUpdate(messages: peerMessages, transaction: transaction)\n"
"            }\n"
"        }\n"
"\n"
"        return addResult\n"
"    }"
)
assert fire_needle in src, "fire anchor missing"
src = src.replace(fire_needle, fire_patch, 1)

impl_api_needle = (
"    public func installStoreOrUpdateMessageAction(peerId: PeerId, action: StoreOrUpdateMessageAction) -> Disposable {\n"
"        let disposable = MetaDisposable()\n"
"        self.queue.async {\n"
"            if self.installedStoreOrUpdateMessageActionsByPeerId[peerId] == nil {\n"
"                self.installedStoreOrUpdateMessageActionsByPeerId[peerId] = Bag()\n"
"            }\n"
"            let index = self.installedStoreOrUpdateMessageActionsByPeerId[peerId]!.add(action)\n"
"            disposable.set(ActionDisposable {\n"
"                self.queue.async {\n"
"                    if let bag = self.installedStoreOrUpdateMessageActionsByPeerId[peerId] {\n"
"                        bag.remove(index)\n"
"                    }\n"
"                }\n"
"            })\n"
"        }\n"
"        return disposable\n"
"    }"
)
impl_api_patch = impl_api_needle + (
"\n\n    public func installGlobalStoreOrUpdateMessageAction(action: StoreOrUpdateMessageAction) -> Disposable {\n"
"        let disposable = MetaDisposable()\n"
"        self.queue.async {\n"
"            let index = self.installedGlobalStoreOrUpdateMessageActions.add(action)\n"
"            disposable.set(ActionDisposable {\n"
"                self.queue.async {\n"
"                    self.installedGlobalStoreOrUpdateMessageActions.remove(index)\n"
"                }\n"
"            })\n"
"        }\n"
"        return disposable\n"
"    }"
)
assert impl_api_needle in src, "impl api anchor missing"
# Only patch the first occurrence (the PostboxImpl one).
src = src.replace(impl_api_needle, impl_api_patch, 1)

wrapper_needle = (
"    public func installStoreOrUpdateMessageAction(peerId: PeerId, action: StoreOrUpdateMessageAction) -> Disposable {\n"
"        let disposable = MetaDisposable()\n"
"\n"
"        self.impl.with { impl in\n"
"            disposable.set(impl.installStoreOrUpdateMessageAction(peerId: peerId, action: action))\n"
"        }\n"
"\n"
"        return disposable\n"
"    }"
)
wrapper_patch = wrapper_needle + (
"\n\n    public func installGlobalStoreOrUpdateMessageAction(action: StoreOrUpdateMessageAction) -> Disposable {\n"
"        let disposable = MetaDisposable()\n"
"\n"
"        self.impl.with { impl in\n"
"            disposable.set(impl.installGlobalStoreOrUpdateMessageAction(action: action))\n"
"        }\n"
"\n"
"        return disposable\n"
"    }"
)
assert wrapper_needle in src, "wrapper anchor missing"
src = src.replace(wrapper_needle, wrapper_patch, 1)

open(p, "w").write(src)
PY
else
  echo "$POSTBOX already patched"
fi


# ---- (3) sqlcipher amalgamation upgrade ----
# Upstream telegram-ios vendors sqlcipher 3.33.0 which predates the FTS5
# trigram tokenizer (added in SQLite 3.34). Seoyu's Korean-aware FTS5
# tables require trigram. Overwrite the submodule's amalgamation with
# the 4.6.1 build we vendor under vendor/sqlcipher-4.6.1/.
SQLCIPHER_SRC_DIR=submodules/telegram-ios/submodules/sqlcipher/Sources
SQLCIPHER_HDR_DIR=submodules/telegram-ios/submodules/sqlcipher/PublicHeaders/sqlcipher
VENDORED=vendor/sqlcipher-4.6.1
CURRENT_SQLITE_VER=$(grep -m1 '^#define SQLITE_VERSION ' "$SQLCIPHER_SRC_DIR/sqlite3.c" | awk -F'"' '{print $2}')
if [[ "$CURRENT_SQLITE_VER" != "3.46.1" ]]; then
  echo "Upgrading sqlcipher amalgamation (was SQLite $CURRENT_SQLITE_VER -> 3.46.1)"
  cp "$VENDORED/sqlite3.c"        "$SQLCIPHER_SRC_DIR/sqlite3.c"
  cp "$VENDORED/sqlite3.h"        "$SQLCIPHER_HDR_DIR/sqlite3.h"
  cp "$VENDORED/sqlite3ext.h"     "$SQLCIPHER_HDR_DIR/sqlite3ext.h"
  cp "$VENDORED/sqlite3session.h" "$SQLCIPHER_HDR_DIR/sqlite3session.h"
else
  echo "sqlcipher amalgamation already at SQLite 3.46.1"
fi

echo "done"
