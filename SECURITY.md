# Security Policy

## How 텔레그램 한국어 검색 Handles Security

- **Session files** are encrypted with AES-256-GCM. The encryption key is stored in macOS Keychain.
- **All data is local-only**. No data is transmitted to external servers (except Telegram's own MTProto API for message fetching).
- **Telegram API credentials** (API ID/Hash) are bundled in the binary, which is standard practice for Telegram clients per their [Terms of Service](https://core.telegram.org/api/obtaining_api_id).
- **Network traffic** uses MTProto encryption provided by the grammers library.

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT** open a public issue
2. Email the maintainer directly at jcs25822@gmail.com
3. Include a description of the vulnerability and steps to reproduce
4. Allow reasonable time for a fix before public disclosure

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |
