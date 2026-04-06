# 텔레그램 한국어 검색

> 텔레그램 그룹/채널 메시지를 **한국어 부분 검색**으로 찾아주는 macOS 데스크톱 앱

텔레그램 데스크톱에는 한국어 부분 검색이 안 됩니다. 이 앱은 텔레그램 메시지를 로컬에 저장하고, **한국어 부분 매칭**과 **띄어쓰기 무시 검색**을 지원합니다. 글로벌 단축키로 어디서든 바로 검색하고, 결과를 클릭하면 텔레그램 데스크톱의 해당 메시지로 바로 이동합니다.

## Features

- **한국어 부분 검색** -- "삼성"으로 검색하면 "삼성전자", "삼성 전자" 모두 찾음
- **띄어쓰기 무시** -- "삼성전자"와 "삼성 전자"를 동일하게 검색
- **영어 검색** -- 영어 부분 문자열 검색도 동일하게 지원
- **빠른 검색** -- FTS5 인덱스 기반, 300ms 이내 응답
- **글로벌 단축키** -- `Cmd+Shift+F`로 어디서든 검색창 열기/닫기
- **메시지 미리보기** -- 검색 결과 클릭 시 메시지 전문 모달 표시, "텔레그램에서 열기" 버튼
- **로컬 전용** -- 모든 데이터는 내 맥에만 저장. 외부 서버 없음
- **백그라운드 동기화** -- 메시지 수집 중에도 검색 가능
- **위키 자동 생성** -- LLM이 메시지를 분류하여 트렌딩 토픽 + 이중언어 위키 문서 자동 생성
- **트렌딩 대시보드** -- 채널 전체에서 화제인 토픽을 한눈에 확인
- **Codex CLI 연동** -- OpenAI API 키 불필요, ChatGPT 구독으로 바로 사용

## Screenshots

_준비 중_

---

## How to Use

### 1. Install

[GitHub Releases](https://github.com/sskys18/telegram-korean-search/releases)에서 최신 `.dmg` 파일을 다운로드하여 설치합니다.

> **참고**: 이 앱은 코드 서명이 되어 있지 않습니다. 처음 실행 시 앱을 우클릭 → "열기" → "열기" 클릭하면 됩니다. 또는 터미널에서:
> ```bash
> xattr -cr /Applications/텔레그램\ 한국어\ 검색.app
> ```

### 2. Telegram API Key 발급

앱에서 텔레그램에 로그인하려면 API 키가 필요합니다.

1. [my.telegram.org](https://my.telegram.org)에 접속
2. 텔레그램 계정으로 로그인
3. **API development tools** 클릭
4. 앱 이름과 설명을 입력하고 생성
5. `api_id`와 `api_hash`를 복사해 둠

### 3. Login

1. 앱을 실행하면 로그인 화면이 나옴
2. 위에서 발급받은 `api_id`와 `api_hash` 입력
3. 전화번호 입력 (국가코드 포함, 예: `+821012345678`)
4. 텔레그램으로 온 인증 코드 입력
5. 2단계 인증을 설정한 경우 비밀번호 입력

### 4. Message Collection

로그인 후 자동으로 텔레그램 그룹/채널 메시지를 수집합니다. 수집 중에도 검색이 가능합니다.

### 5. Search

- `Cmd+Shift+F`로 검색창을 열고 검색어 입력
- 검색 결과를 클릭하면 텔레그램 데스크톱의 해당 메시지로 이동

---

## Build from Source

릴리즈 대신 소스에서 직접 빌드하려면 아래 단계를 따릅니다.

### Prerequisites

- macOS 12+
- [Rust](https://rustup.rs/) 1.75+
- [Bun](https://bun.sh/) 1.0+

### Build

```bash
# 저장소 클론
git clone https://github.com/sskys18/telegram-korean-search.git
cd telegram-korean-search

# 프론트엔드 의존성 설치
bun install

# 개발 모드 실행
cargo tauri dev

# 프로덕션 빌드
cargo tauri build
```

---

## Tech Stack

| 구성 | 기술 |
|------|------|
| Backend | Rust, Tauri v2 |
| Frontend | React + TypeScript + Vite |
| Telegram | grammers (MTProto) |
| Search | SQLite FTS5 trigram tokenizer |
| Wiki/LLM | Codex CLI (o4-mini classification, gpt-5.4 summaries) |
| Storage | SQLite (WAL mode) |
| Security | AES-256-GCM session encryption, macOS Keychain |

## Architecture

자세한 설계 문서는 [docs/architecture.md](docs/architecture.md)를 참고하세요.

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md)를 참고하세요.

## License

[MIT](LICENSE)
