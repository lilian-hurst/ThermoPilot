import SwiftUI

struct MenuBarView: View {
    @Environment(EngineClient.self) private var client
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let s = client.snapshot {
                HStack {
                    Image(systemName: Thermal.levelIcon(s.advice.level)).foregroundStyle(Thermal.levelColor(s.advice.level))
                    Text(s.advice.title).font(.headline)
                }
                Text(s.advice.message).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                Divider()
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                    ForEach(["cpu_p", "cpu_e", "gpu", "soc"].compactMap { s.group($0) }) { g in
                        GridRow {
                            Text(g.name).font(.caption)
                            Text(String(format: "%.0f °C", g.max)).font(.caption.bold().monospacedDigit())
                                .foregroundStyle(Thermal.color(g.max, warn: g.warn, crit: g.crit))
                        }
                    }
                    if let b = s.battery {
                        GridRow {
                            Text(tr("Batterie", "Battery")).font(.caption)
                            Text("\(Int(b.percent)) % · \(s.charge.state)").font(.caption.bold()).lineLimit(1)
                        }
                    }
                    ForEach(s.fans) { f in
                        GridRow {
                            Text(f.name).font(.caption)
                            Text("\(Int(f.rpm)) RPM (\(Int(f.percent)) %)").font(.caption.bold().monospacedDigit())
                        }
                    }
                }
                Divider()
                if s.privileged {
                    HStack {
                        Button("Auto") { client.setMode(.auto) }
                        Button(tr("Équilibré", "Balanced")) { client.setMode(.curve(.balanced)) }
                        Button("Perf.") { client.setMode(.curve(.performance)) }
                        Button("Max") { client.setMode(.manual(100)) }
                    }
                    .controlSize(.small)
                    if s.advice.action != "none" {
                        Button(tr("Appliquer le conseil (\(Int(s.advice.suggestedPercent)) %)", "Apply advice (\(Int(s.advice.suggestedPercent)) %)")) {
                            client.setMode(s.advice.action == "decrease" ? .curve(.balanced) : .manual(s.advice.suggestedPercent))
                        }
                        .buttonStyle(.borderedProminent).tint(Thermal.levelColor(s.advice.level))
                    }
                } else {
                    Text(tr("Lecture seule — installez le helper pour piloter les ventilateurs.", "Read-only — install the helper to control the fans.")).font(.caption).foregroundStyle(.orange)
                }
            } else {
                ProgressView(tr("Connexion…", "Connecting…"))
            }
            Divider()
            HStack {
                Button(tr("Tableau de bord", "Dashboard")) {
                    Dock.show()
                    openWindow(id: "main")
                }
                Spacer()
                LanguageMenu()
                Button(tr("Quitter", "Quit")) { NSApp.terminate(nil) }
            }
        }
        .padding(14)
        .frame(width: 320)
    }
}
