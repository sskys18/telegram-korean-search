# telegram-seoyu

> 한국어 검색이 제대로 되는 macOS Telegram 클라이언트 + 로컬 LLM 위키

[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift) 포크에
Rust 사이드카를 붙여서 전용 한국어 FTS5 인덱스를 운영한다. 글로벌 검색창에
한국어를 입력하면, Telegram 서버 검색 결과와 함께 로컬 인덱스 히트가
병합되어 나타난다.

**상태:** 개발 빌드 공개. 서명 없음, macOS 26 arm64 전용.
최신 릴리즈 → **[v0.4.0-dev](https://github.com/sskys18/telegram-korean-search/releases/latest)**.

[English README](README.md)

## 뭘 고쳤나

Telegram 기본 검색은 영어 중심이다. 한국어 쿼리는 세 가지 방식으로
망가진다. Seoyu는 로컬 인덱스로 세 가지 모두 해결한다.

| 쿼리                  | 원본 Telegram   | telegram-seoyu      |
| --------------------- | --------------- | ------------------- |
| `삼성` (부분 문자열)    | ❌ 매칭 안됨    | ✅ `삼성전자` 찾음   |
| `삼성 전자` (띄어쓰기) | ❌ 매칭 안됨    | ✅ `삼성전자` 찾음   |
| `ㅅㅏㅁ` (자모)        | ❌ 매칭 안됨    | ✅ `삼성` 찾음       |

### 같은 쿼리, 같은 순간, 나란히

쿼리: **`삼성 전재`** (오타 + 띄어쓰기). 왼쪽: Seoyu. 오른쪽: 원본 TelegramSwift.

| telegram-seoyu (이 프로젝트) | 원본 TelegramSwift |
| ---------------------------- | ------------------- |
| ![seoyu](docs/images/search-seoyu.png) | ![upstream](docs/images/search-upstream.png) |
| **10개 이상 채팅방**, 몇 주 범위의 날짜에 걸쳐 히트. `nospace` 컬럼이 `삼성 전재` → `삼성전자`로 정규화해 부분 문자열로 매칭. | 최근 채팅방 **하나**에서만 히트. 띄어쓰기 + 오타에 서버 검색이 복구 못하고, 서버측 최근 기록만 스캔함. |

## 아키텍처

```
┌──────────────────────────────────────────┐
│  TelegramSwift 포크 (AppKit, GPLv2)       │
│   ├── 원본 채팅 UI, 로그인, 동기화           │
│   ├── SearchController 훅                 │
│   └── SeoyuBridge (사이드카 오픈·쿼리,      │
│       Postbox 미러링)                     │
└─────────────────┬────────────────────────┘
                  │ packages/Seoyu FFI
┌─────────────────▼────────────────────────┐
│  telegram-seoyu 사이드카 (Rust, MIT)      │
│   ├── SQLite 미러 (메시지 + FTS5)          │
│   ├── 한국어 정규화기                      │
│   │     (트라이그램 / 자모 / nospace)      │
│   ├── 검색 엔진                           │
│   └── 위키 파이프라인 (codex exec)         │
└──────────────────────────────────────────┘
```

Postbox에 저장되는 모든 메시지는 사이드카 SQLite 스토어에 미러링되고
FTS5의 `content` / `jamo` (분해 자모) / `nospace` (공백 제거) 세 컬럼에
인덱싱된다. 검색은 세 컬럼 모두에 fan-out 후 랭크 병합. 기기 밖으로
나가는 데이터는 TelegramSwift가 원래 보내는 MTProto 트래픽뿐.

## 설치 (DMG 사용)

**요구사항:** macOS 26 (Tahoe) 이상, Apple Silicon.

1. [최신 릴리즈](https://github.com/sskys18/telegram-korean-search/releases/latest)에서
   `Telegram-seoyu.dmg` 다운로드.
2. DMG 마운트하고 **Telegram.app**을 `/Applications`에 드래그.
3. 서명 없는 빌드라 quarantine 플래그 해제 필요:
   ```bash
   xattr -dr com.apple.quarantine /Applications/Telegram.app
   ```
   또는 Finder에서 첫 실행 때만 우클릭 → **열기** → **열기**.
4. 실행하고 전화번호로 로그인 (원본 Telegram처럼).

첫 실행은 기존 메시지를 로컬 FTS 인덱스에 수집하느라 1분 정도 걸린다.
이후 실행은 즉시 뜬다.

## 소스에서 빌드

**요구사항:** macOS 26.4+, Xcode 26.4+, Apple Silicon, 디스크 여유 ~10 GB.

```bash
git clone https://github.com/sskys18/telegram-korean-search.git
cd telegram-korean-search
./scripts/build-dev.sh --run      # 첫 빌드 15~30분, 성공 시 실행
```

서명된 배포용 빌드는 Xcode에서 `Telegram-Mac.xcworkspace`를 열고 본인
Developer ID 팀으로 빌드. Xcode 26의 Swift 드라이버 버그 대응은
[`docs/XCODE26-BLOCKER.md`](docs/XCODE26-BLOCKER.md)의 `scripts/ld-cryptex-shim.sh`
참고 — CLI 빌드를 가능하게 만드는 우회책.

### 사이드카 단독

```bash
cd sidecar
cargo build --release     # tg-seoyu-sidecar 생성
cargo test                # 85 pass, 1 ignored
cargo clippy -- -D warnings
cargo fmt --check
```

## 저장소 구조

```
Telegram-Mac/              Swift 앱 타깃 (원본 + Seoyu/)
Telegram-Mac/Seoyu/        SeoyuBridge + ingest observer
packages/Seoyu/            사이드카 Swift 래퍼 (UniFFI)
Telegram-Mac.xcworkspace   Xcode로 열기

sidecar/                   Rust 크레이트: 검색, 스토어, 위키, 보안
scripts/                   build-dev.sh, ld-cryptex-shim.sh, …
docs/                      핸드오프, 블로커, 설계 노트
```

## 데이터 위치

```
~/Library/Application Support/telegram-korean-search/
    tg-korean-search.db       SQLite + FTS5 (메시지, 위키, 큐)
```

TelegramSwift의 Postbox는 별도 디렉토리에 있고, 사이드카는 건드리지 않음.

## 삭제

```bash
rm -rf /Applications/Telegram.app
rm -rf ~/Library/Application\ Support/telegram-korean-search
```

Telegram 계정과 서버 메시지 기록은 그대로 남음. 로컬 인덱스만 제거됨.

## 귀속

이 프로젝트는
[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift) 포크다.
저장소 루트의 Swift 소스, Xcode 워크스페이스, `submodules/`는 전부 원본
작업물이며 **GPLv2** 라이선스. 사이드카는 새로 작성한 코드로 **MIT**
라이선스.

원본 포크 요구사항에 따라:

1. 자체 Telegram API ID 사용 (첫 실행 시 설정).
2. "Telegram"이라는 이름을 쓰지 않음. 앱과 저장소 이름은 `telegram-seoyu`.
3. Telegram 표준 로고 사용 안함.
4. Telegram의
   [MTProto 보안 가이드라인](https://core.telegram.org/mtproto/security_guidelines)
   준수.
5. GPLv2에 따라 소스 공개.

## 라이선스

- Swift 셸 + 모든 원본 코드: **GPLv2** ([LICENSE](LICENSE))
- `sidecar/` Rust 크레이트: **MIT** (`sidecar/Cargo.toml`)

## 범위

개인용, 로컬 전용, macOS 전용. 개발 빌드는 서명 없음. 서명 배포는 Apple
Developer ID 필요하지만 아직 없음.
