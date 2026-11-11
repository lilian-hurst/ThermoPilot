# Contributing to ThermoPilot

Thanks for your interest! 🎉 ThermoPilot is a young project, and the goal is to make it better together: more Macs supported, more languages, more useful features.
All contributions are welcome: hardware reports, bug reports, ideas, translations, documentation and code.

## 🧪 The most useful thing right now: hardware reports

ThermoPilot has only been tested on an **M3 Max MacBook Pro**. Apple changes SMC key names between chips, so the fastest way to help is to tell us how it behaves on **your** Mac:

1. Download the [latest release](../../releases/latest) and open the dashboard.
2. Run the engine's dump (no admin rights needed):
   ```bash
   /Applications/ThermoPilot.app/Contents/MacOS/fanengine dump > smc-dump.txt
   /Applications/ThermoPilot.app/Contents/MacOS/fanengine status > status.json
   ```
3. [Open a hardware report](../../issues/new?template=hardware_report.yml) and attach both files.

## 🛠️ Development setup

Requirements: macOS 15+, Apple Silicon, **Rust** (stable, via [rustup](https://rustup.rs)) and **Xcode 16+ / Swift 6**.

```bash
git clone https://github.com/lilian-hurst/ThermoPilot.git
cd ThermoPilot
./build.sh                 # builds the Rust engine + SwiftUI app into build/ThermoPilot.app
open build/ThermoPilot.app
```

- Without the root helper, the app starts the engine in **read-only** mode: enough to work on the UI, sensors, battery and usage features.
- To test fan or charge control, install the helper from the app (status pill › **Install root helper…**). **Reinstall it after every engine change**: the installed helper is a copy of the binary.
- `cd engine && cargo test` runs the engine's unit tests.
- `engine/target/release/fanengine bench` shows the cost of every read. Please keep ThermoPilot light.

## 🗂️ Project structure

| Path | What's inside |
|---|---|
| `app/Sources/ThermoPilot/` | SwiftUI app: `DashboardView` (thermal), `BatteryView` + `UsageView` (battery), `MenuBarView`, `EngineClient` (socket client), `Models` (JSON models + `tr()` i18n helper) |
| `engine/src/smc.rs` | AppleSMC access through IOKit (read/write keys, type decoding) |
| `engine/src/sensors.rs`, `fans.rs`, `control.rs` | Sensor groups, fan read/write, fan modes, curves and advisor |
| `engine/src/battery.rs`, `charge.rs`, `power.rs` | Battery info, charge limiter, `pmset` settings, sleep/wake hooks |
| `engine/src/energy.rs`, `usage.rs`, `battery_advice.rs` | Per-process energy, usage recorder and drain diagnostics, tips |
| `engine/src/engine.rs`, `server.rs` | Sampling loop (adaptive cadence) and the JSON-lines Unix socket API |
| `engine/src/i18n.rs` | Engine language (`tr!` / `trs!` macros) |
| `scripts/` | Helper install/uninstall, release and icon scripts |

## 🌍 Translations

UI strings use `tr("Français", "English")` in Swift and `tr!(…)` / `trs!(…)` in Rust. Adding a third language means moving to string catalogs, and that would be a great contribution. Open an issue first so we can agree on the approach.

## ✅ Pull request checklist

- One topic per PR, with a clear description of the change (screenshots for UI changes).
- `./build.sh` succeeds and `cd engine && cargo test` passes.
- Anything that **writes to the SMC** must keep the safety guarantees: give control back to macOS on quit, sleep and helper stop, and force the fans to max above 100 °C.
- Keep the background cost low: no polling loops or spawned processes in the hot path.
- Both languages updated for new UI text (French can be approximate, it will be reviewed).

## 💡 Good first issues

- Support for sensor/fan keys of another chip (from a hardware report)
- Launch at login (`SMAppService`)
- Battery history CSV export
- Small UI polish and accessibility improvements

Look for the [`good first issue`](../../issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) label, or just ask in an issue. Happy to help you get started!

## Code of conduct

Be kind and constructive. We're all here to make Macs cooler. 🧊
