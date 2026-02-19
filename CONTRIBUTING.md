# Contributing to 텔레그램 한국어 검색

Thank you for your interest in contributing! Here's how you can help.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/telegram-korean-search.git`
3. Create a branch: `git checkout -b feature/your-feature`
4. Make your changes
5. Test your changes: `cargo test` and `cargo clippy`
6. Commit: `git commit -m "Add your feature"`
7. Push: `git push origin feature/your-feature`
8. Open a Pull Request

## Development Setup

```bash
# Prerequisites: Rust 1.75+, Bun 1.0+
bun install
cargo tauri dev
```

## Code Style

- **Rust**: Follow standard Rust conventions. Run `cargo fmt` and `cargo clippy` before committing.
- **TypeScript/React**: Use consistent formatting. Run `bun run lint` if available.
- Keep changes focused -- one feature or fix per PR.

## Reporting Issues

- Use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md) for bugs
- Use the [feature request template](.github/ISSUE_TEMPLATE/feature_request.md) for new ideas
- Check existing issues before creating a new one

## Pull Request Guidelines

- Keep PRs small and focused
- Include a description of what changed and why
- Add tests for new functionality when possible
- Ensure CI passes before requesting review

### Release Labels

When a PR is merged to `main`, a new version is automatically tagged and released. Add one of these labels to your PR to control the version bump:

| Label | Bump | Example |
|-------|------|---------|
| `major` | Major | `0.2.0` -> `1.0.0` |
| `minor` | Minor | `0.2.0` -> `0.3.0` |
| _(no label)_ | Patch | `0.2.0` -> `0.2.1` |

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Please be respectful and constructive.
