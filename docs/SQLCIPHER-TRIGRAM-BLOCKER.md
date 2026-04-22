# sqlcipher FTS5 trigram blocker — RESOLVED 2026-04-22

## Symptom

The Seoyu ingest observer logs `no such tokenizer: trigram` for every
batch of messages it tries to mirror into the sidecar store. The
existing Seoyu database still answers simple queries but never grows,
and the chat-list search merge has nothing new to surface.

## Root cause

The TelegramSwift fork links `submodules/telegram-ios/submodules/sqlcipher`
as a static library. `sqlcipher/Sources/sqlite3.c` vendors **SQLite
3.33.0**. The FTS5 trigram tokenizer did not land until SQLite 3.34
(2020). sqlcipher's Swift Package enables `SQLITE_ENABLE_FTS5` but does
not ship the trigram source files, so `CREATE VIRTUAL TABLE … USING
fts5(… tokenize='trigram')` succeeds at migration time but any subsequent
insert fails because the tokenizer is not registered.

At link time sqlcipher's statically defined `sqlite3_*` symbols win over
the ones in `/usr/lib/libsqlite3.dylib` (macOS 26 system SQLite 3.51
which *does* ship trigram). The Rust sidecar's `sqlite` crate resolves
its calls to sqlcipher, not to the system dylib, because both are
visible in the same binary and static symbols take precedence.

Verified:

```
$ grep '^#define SQLITE_VERSION ' submodules/telegram-ios/submodules/sqlcipher/Sources/sqlite3.c
#define SQLITE_VERSION        "3.33.0"
$ grep -ci trigram submodules/telegram-ios/submodules/sqlcipher/Sources/sqlite3.c
0
$ nm …/Telegram.debug.dylib | grep 'T _sqlite3_libversion'
…  T _sqlite3_libversion  # defined, i.e. sqlcipher wins the link
```

## Ways forward

1. **Upgrade sqlcipher to 4.5+**. Replace `sqlite3.c` / `sqlite3.h` in
   the submodule with the amalgamation from a sqlcipher release that
   vendors SQLite ≥ 3.34 (sqlcipher 4.5.0 ships 3.39.4). Re-apply via
   `scripts/patch-submodules.sh`. Preferred: keeps the Korean search
   design intact.
2. **Drop trigram from the sidecar schema** and switch to a tokenizer
   sqlcipher 3.33 already ships (`unicode61` with `remove_diacritics 2`).
   Substring / 초성 / nospace queries would need to fall back to `LIKE`
   for short inputs (crate already has `search_messages_like_*`).
   Cheaper than (1) but regresses the plan documented in CLAUDE.md.
3. **Namespace rename the sidecar's SQLite** so the two copies do not
   clash (static rusqlite with a C-level prefix rewrite). Complex and
   nobody does this.

## Fix

Option 1 was taken. We vendor the sqlcipher 4.6.1 amalgamation (SQLite
3.46.1, trigram included) at `vendor/sqlcipher-4.6.1/` and
`scripts/patch-submodules.sh` overwrites the submodule's `sqlite3.c`,
`sqlite3.h`, `sqlite3ext.h`, `sqlite3session.h` with it whenever the
submodule is back on upstream 3.33.0. The patch is idempotent and keyed
on the `#define SQLITE_VERSION` line so repeated runs are no-ops.

## Verification

After rebuild on the new amalgamation, launching the fork and letting
Postbox sync caused `tg-korean-search.db` to grow from 190,330 rows to
191,969 rows in roughly a minute, with the max timestamp moving from
2026-04-17 (Tauri-era indexing cutoff) to the current instant. No
`no such tokenizer: trigram` errors in the log. The ingest observer
is now the authoritative source for new rows.
