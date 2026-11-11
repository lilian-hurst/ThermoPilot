# Security Policy

ThermoPilot installs a small **root helper** (LaunchDaemon) that writes to the Mac's SMC to control fans and charging, so security reports are taken seriously.

## Supported versions

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Older releases | ❌ (please update) |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report it privately through GitHub: **[Security › Report a vulnerability](../../security/advisories/new)**.
Include the affected version, your Mac model / macOS version, and steps to reproduce.

You can expect an acknowledgement within **7 days**. Once a fix is released, you will be credited in the release notes (unless you prefer not to be).

## Scope

Especially relevant:
- The root helper (`/Library/PrivilegedHelperTools/com.lilian.fanengine`) and its Unix socket (`/var/run/fanengine.sock`): access control (only root and the installing user may connect), command validation, privilege escalation.
- The install / uninstall scripts (`scripts/`).
- Anything that could leave fans stopped or the power adapter disabled.

## Design safeguards

- The socket accepts only root and the user who installed the helper (`getpeereid`).
- Fan control is returned to macOS when the app quits, before sleep, and when the helper stops; fans are forced to max above 100 °C.
- The power adapter is re-enabled before sleep, at ≤ 15 % battery and when the helper stops.
- Only a whitelist of `pmset` keys and value ranges can be changed.
