# Install

Two paths: download a prebuilt DMG (fastest) or build from source.

## Prebuilt DMG

**Requirements:** macOS 26 (Tahoe) or newer, Apple Silicon.

1. Grab `Telegram-seoyu.dmg` from the
   [latest release](https://github.com/sskys18/telegram-korean-search/releases/latest).
2. Mount the DMG and drag **Telegram.app** into `/Applications`.
3. Clear the quarantine attribute (required — the build is unsigned):
   ```bash
   xattr -dr com.apple.quarantine /Applications/Telegram.app
   ```
   Alternative: right-click in Finder → **Open** → **Open** in the
   confirmation dialog. After the first launch, normal double-click works.
4. Launch. Sign in with your Telegram phone number.

First launch takes a minute while the sidecar ingests your existing
messages into the FTS index.

## Build from source

**Requirements:** macOS 26.4+, Xcode 26.4+, Apple Silicon, ~10 GB free.

```bash
git clone https://github.com/sskys18/telegram-korean-search.git
cd telegram-korean-search
./scripts/build-dev.sh --run       # build + launch
./scripts/build-dev.sh --dmg       # build + package dist/Telegram-seoyu.dmg
```

The script handles:

- Submodule init (`git submodule update --init --recursive`)
- Firebase / GoogleAppMeasurement shallow-framework fixup
  (`scripts/fix-shallow-frameworks.sh`)
- Xcode 26 Metal-cryptex linker workaround via `scripts/ld-cryptex-shim.sh`
- Unsigned arm64 Debug build into
  `~/Library/Developer/Xcode/DerivedData/Telegram-Mac-dev/`

See [`docs/XCODE26-BLOCKER.md`](docs/XCODE26-BLOCKER.md) for why the shim
exists and what breaks without it.

### Signed, distributable build

Open `Telegram-Mac.xcworkspace` in Xcode, pick your Developer ID team,
and build normally. Xcode's internal build driver is mostly unaffected by
the Metal-cryptex bug; if you hit it from GUI after a system update, fall
back to `./scripts/build-dev.sh` and sign the output manually.

### Sidecar in isolation

```bash
cd sidecar
cargo build --release     # produces tg-seoyu-sidecar
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## Uninstall

```bash
rm -rf /Applications/Telegram.app
rm -rf ~/Library/Application\ Support/telegram-korean-search
```

Your Telegram account and server-side message history are not affected;
only the local FTS index is removed.

## Forking this repo

This fork is set up with its own bundle identifier, Telegram API ID, and
app name per upstream's fork requirements. If you fork further:

1. Replace mentions of `com.seoyu.telegram-seoyu` (bundle id) and the
   corresponding Team ID. Team ID lives in `Telegram-Mac/common.xcconfig`
   and in Xcode build settings.
2. Obtain your own [Telegram API ID](https://core.telegram.org/api/obtaining_api_id)
   and replace `apiId` / `apiHash` in `Telegram-Mac/Config.swift`.
3. Replace `SUFEED_URL` and `APPCENTER_SECRET` in the `.xcconfig` files
   if you want your own update feed and crash reporting.
4. Change the app name and icon so you comply with the Telegram
   trademark guidelines.

Upstream build instructions (for reference) live at
<https://github.com/overtake/TelegramSwift/blob/master/INSTALL.md>.
