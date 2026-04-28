# Security Policy

## Supported versions

YapStack is in alpha. Only the **latest published release** receives security updates. Older releases (including alpha pre-releases) are not patched.

| Version | Supported |
|---------|-----------|
| Latest release | ✅ |
| Older releases | ❌ |

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues, discussions, or pull requests.**

Instead, report them privately using one of:

1. **GitHub private security advisory** — Preferred. Use the [Report a vulnerability](https://github.com/hookr-dev/YapStack/security/advisories/new) button on the Security tab.
2. **Email** — Send to `clayton@claytonn.com` with `[YapStack Security]` in the subject line.

Please include:

- A description of the issue and its impact
- Steps to reproduce, or a proof-of-concept
- The YapStack version you tested against
- Your platform (macOS Apple Silicon / Intel / Windows / Linux)
- Whether you've already disclosed this elsewhere

## Response timeline

This is a small project. Expect:

- **Acknowledgement** within 7 days.
- **Triage + initial response** within 14 days.
- **Fix or mitigation** depending on severity. Critical issues prioritized.

## Scope

In scope:

- The YapStack desktop application (Tauri shell, Rust backend, React frontend).
- Build/release pipeline (`.github/workflows/`).
- Bundled or downloaded models / signed binaries.

Out of scope:

- Vulnerabilities in upstream dependencies — please report those to the dependency maintainer (we'll pull in fixes via Dependabot once they ship).
- Issues that require physical access to an unlocked device.
- Social engineering of maintainers or users.

## Disclosure

We follow [coordinated disclosure](https://en.wikipedia.org/wiki/Coordinated_vulnerability_disclosure). Once a fix ships in a public release, the original reporter will be credited in the GitHub Security Advisory unless they prefer to remain anonymous.
