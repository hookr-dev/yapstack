# Contributing to YapStack

Thanks for your interest in contributing. YapStack is built openly with AI-assisted development as a first-class part of the workflow — bring whatever tools you like. We care about correctness, design clarity, and tests, not provenance.

## Ways to contribute

- **Bug reports** — Open a [bug report issue](https://github.com/hookr-dev/YapStack/issues/new?template=bug_report.yml) with reproduction steps, your platform, and any relevant logs.
- **Feature requests** — Open a [feature request](https://github.com/hookr-dev/YapStack/issues/new?template=feature_request.yml). Describe the problem first, then the proposed solution.
- **Pull requests** — See workflow below.
- **Documentation** — Typo fixes, clarifications, and examples are welcome via PR with no prior discussion.

## Development setup

Prerequisites:

- **Rust** ≥ 1.77.2 — install via [rustup](https://rustup.rs)
- **Node.js** ≥ 22
- **pnpm** — `corepack enable && corepack prepare pnpm@latest-10 --activate`
- **cmake** — required only if building with the `whisper` feature flag

```bash
git clone https://github.com/hookr-dev/YapStack.git
cd YapStack
pnpm install
pnpm tauri dev
```

See [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for build details, feature flags, and platform-specific notes (including local Windows builds, which are unofficial).

## Pull request workflow

1. **Fork** the repository.
2. **Branch** from `main`. Naming conventions: `feat/<short-name>`, `fix/<short-name>`, `chore/<short-name>`, `refactor/<short-name>`.
3. **Make your change.** Keep the diff focused — one logical change per PR.
4. **Run `pnpm check` locally before submitting.** This runs `cargo build --all`, `cargo test --all`, `cargo fmt --check`, `cargo clippy -D warnings`, plus frontend `typecheck`, `lint`, and `vitest`. CI runs the same suite on macOS only.
5. **Update [`CHANGELOG.md`](CHANGELOG.md)** under `## [Unreleased]` if your change is user-visible. See the discipline guide in [`CLAUDE.md`](CLAUDE.md) for what counts.
6. **Open the PR.** The PR template will prompt you for a summary, motivation, and a test plan.

CI must pass before review. PRs that fail CI will not be reviewed until green.

## Coding conventions

- See [`docs/PRINCIPLES.md`](docs/PRINCIPLES.md) for design and testing posture.
- Default to writing no comments. Add one only when the **why** is non-obvious.
- Don't introduce abstractions speculatively. Three similar lines is better than a premature abstraction.
- Trust internal code and framework guarantees. Validate at system boundaries (user input, external APIs) only.
- For UI/frontend changes, exercise the feature in a browser before reporting it as done. Type-checks and tests verify code correctness, not feature correctness.

## Platform support reminder

YapStack officially supports macOS (Apple Silicon primary, Intel best-effort). Windows builds are experimental and not produced by CI/CD — patches that affect Windows-specific code paths are welcome but won't block macOS-targeted PRs. See [`README.md`](README.md#platform-support) for the full matrix.

## Reporting security issues

Do **not** report security issues via public GitHub issues. See [`SECURITY.md`](SECURITY.md) for the disclosure process.

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you agree to abide by its terms.

## License

By contributing, you agree that your contributions will be licensed under the same [GNU Affero General Public License v3.0](LICENSE) that covers the project.
