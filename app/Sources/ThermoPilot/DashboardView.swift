import Charts
import SwiftUI

struct DashboardView: View {
    @Environment(EngineClient.self) private var client

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HeaderView()
                if let s = client.snapshot {
                    AdviceBanner(snapshot: s)
                    if let err = client.commandError ?? s.lastError {
                        Label(err, systemImage: "xmark.octagon.fill")
                            .foregroundStyle(.red).font(.callout)
                    }
                    TemperatureGrid(snapshot: s)
                    HStack(alignment: .top, spacing: 16) {
                        FansPanel(snapshot: s).frame(maxWidth: .infinity)
                        ControlPanel(snapshot: s).frame(maxWidth: .infinity)
                    }
                    HistoryCharts(history: client.history, fans: s.fans)
                    OtherSensors(snapshot: s)
                    RawSensors(sensors: s.sensors)
                } else {
                    ConnectingView()
                }
            }
            .padding(20)
        }
        .background(.background.secondary)
    }
}

struct Card<Content: View>: View {
    let title: String
    var icon: String? = nil
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let icon { Label(title, systemImage: icon).font(.headline) } else { Text(title).font(.headline) }
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.quaternary))
    }
}

struct ConnectingView: View {
    @Environment(EngineClient.self) private var client
    var body: some View {
        VStack(spacing: 12) {
            if client.state == .outdated {
                Image(systemName: "arrow.triangle.2.circlepath.circle").font(.largeTitle).foregroundStyle(.orange)
                Text(tr("Le helper root installé date d'une version précédente du moteur.", "The installed root helper is from an older version of the engine."))
                Button(tr("Réinstaller le helper root…", "Reinstall root helper…")) { client.installHelper() }.buttonStyle(.borderedProminent)
            } else if case .failed(let msg) = client.state {
                Image(systemName: "exclamationmark.triangle").font(.largeTitle).foregroundStyle(.orange)
                Text(msg)
                Button(tr("Réessayer", "Retry")) { client.connect() }
            } else {
                ProgressView()
                Text(tr("Connexion au moteur Rust…", "Connecting to the Rust engine…")).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 300)
    }
}

// MARK: - En-tête

struct HeaderView: View {
    @Environment(EngineClient.self) private var client
    var body: some View {
        HStack(alignment: .center) {
            Image(systemName: "fan.fill").font(.system(size: 30)).foregroundStyle(.tint)
            VStack(alignment: .leading) {
                Text("ThermoPilot").font(.title.bold())
                Text(client.snapshot.map { "\($0.cpuModel) · \($0.hwModel)" } ?? "MacBook Pro")
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if let s = client.snapshot {
                Stat(label: tr("Puissance", "Power"), value: s.powerW.map { String(format: "%.0f W", $0) } ?? "—")
                Stat(label: "CPU", value: s.cpuUsage.map { String(format: "%.0f %%", $0) } ?? "—")
                Stat(label: "GPU", value: s.gpuUsage.map { String(format: "%.0f %%", $0) } ?? "—")
            }
            LanguageMenu()
            ConnectionPill()
        }
    }
}

/// Choix de la langue de l'interface (Système / Français / English).
struct LanguageMenu: View {
    @Environment(EngineClient.self) private var client
    var body: some View {
        Menu {
            Picker(tr("Langue", "Language"), selection: Binding(get: { client.languageChoice }, set: { client.setLanguage($0) })) {
                ForEach(Lang.choices, id: \.id) { c in Text(c.label).tag(c.id) }
            }
            .pickerStyle(.inline)
        } label: {
            Image(systemName: "globe")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help(tr("Langue", "Language"))
    }
}

struct Stat: View {
    let label: String
    let value: String
    var body: some View {
        VStack(alignment: .trailing, spacing: 2) {
            Text(value).font(.title3.monospacedDigit().bold())
            Text(label).font(.caption).foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10)
    }
}

struct ConnectionPill: View {
    @Environment(EngineClient.self) private var client
    var body: some View {
        let (text, color, icon): (String, Color, String) = switch client.state {
        case .root: (tr("Contrôle actif", "Control active"), .green, "lock.open.fill")
        case .readOnly: (tr("Lecture seule", "Read-only"), .orange, "lock.fill")
        case .connecting: (tr("Connexion…", "Connecting…"), .gray, "hourglass")
        case .outdated: (tr("Helper obsolète", "Helper outdated"), .orange, "arrow.triangle.2.circlepath")
        case .failed: (tr("Hors ligne", "Offline"), .red, "bolt.horizontal.circle")
        }
        Menu {
            if client.helperInstalled {
                Button(tr("Réinstaller le helper root…", "Reinstall root helper…")) { client.installHelper() }
                Button(tr("Désinstaller le helper root…", "Uninstall root helper…"), role: .destructive) { client.uninstallHelper() }
            } else {
                Button(tr("Installer le helper root…", "Install root helper…")) { client.installHelper() }
            }
            Button(tr("Reconnecter", "Reconnect")) { client.reconnect() }
        } label: {
            Label(text, systemImage: icon)
        }
        .menuStyle(.button)
        .buttonStyle(.bordered)
        .tint(color)
        .fixedSize()
    }
}

// MARK: - Conseil

struct AdviceBanner: View {
    @Environment(EngineClient.self) private var client
    let snapshot: Snapshot

    var body: some View {
        let a = snapshot.advice
        let color = Thermal.levelColor(a.level)
        HStack(alignment: .center, spacing: 14) {
            Image(systemName: Thermal.levelIcon(a.level))
                .font(.system(size: 28)).foregroundStyle(color).frame(width: 40)
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(a.title).font(.headline)
                    if snapshot.safetyOverride {
                        Text(tr("SÉCURITÉ : ventilateurs forcés au max", "SAFETY: fans forced to max")).font(.caption.bold())
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(.red, in: Capsule()).foregroundStyle(.white)
                    }
                }
                Text(a.message).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if a.action != "none" {
                    Text(tr("Conseillé : \(Int(a.suggestedPercent)) % · ", "Recommended: \(Int(a.suggestedPercent)) % · ") + a.suggestedRpm.map { "\(Int($0)) RPM" }.joined(separator: " / "))
                        .font(.caption.monospacedDigit()).foregroundStyle(color)
                }
            }
            Spacer()
            if a.action != "none" {
                if snapshot.privileged {
                    Button {
                        client.setMode(a.action == "decrease" ? .curve(.balanced) : .manual(a.suggestedPercent))
                    } label: {
                        Label(a.action == "decrease" ? tr("Passer en Équilibré", "Switch to Balanced") : tr("Appliquer \(Int(a.suggestedPercent)) %", "Apply \(Int(a.suggestedPercent)) %"), systemImage: "wand.and.stars")
                    }
                    .buttonStyle(.borderedProminent).tint(color)
                } else {
                    Button(tr("Installer le helper pour agir", "Install the helper to act")) { client.installHelper() }
                        .buttonStyle(.bordered)
                }
            }
        }
        .padding(16)
        .background(color.opacity(0.12), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(color.opacity(0.5)))
    }
}

// MARK: - Températures

struct TemperatureGrid: View {
    let snapshot: Snapshot
    private let main = ["cpu_p", "cpu_e", "gpu", "soc"]

    var body: some View {
        HStack(spacing: 12) {
            ForEach(main.compactMap { snapshot.group($0) }) { g in
                TempGauge(group: g)
            }
            HotspotCard(snapshot: snapshot)
        }
    }
}

struct TempGauge: View {
    let group: SensorGroup
    var body: some View {
        let color = Thermal.color(group.max, warn: group.warn, crit: group.crit)
        VStack(spacing: 8) {
            Gauge(value: min(group.max, 110), in: 20...110) {
                EmptyView()
            } currentValueLabel: {
                Text("\(Int(group.max.rounded()))°").font(.title2.bold().monospacedDigit())
            }
            .gaugeStyle(.accessoryCircular)
            .tint(Gradient(colors: [.green, .yellow, .orange, .red]))
            .scaleEffect(1.5)
            .frame(width: 90, height: 90)
            Text(group.name).font(.subheadline.bold()).multilineTextAlignment(.center).lineLimit(2)
            Text(tr("moy. \(String(format: "%.0f", group.avg))° · \(group.count) capteurs", "avg \(String(format: "%.0f", group.avg))° · \(group.count) sensors"))
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity)
        .background(.background, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(color.opacity(0.6), lineWidth: 1.5))
    }
}

struct HotspotCard: View {
    let snapshot: Snapshot
    var body: some View {
        let color = Thermal.color(snapshot.hotspot)
        VStack(spacing: 6) {
            Text(tr("Point chaud", "Hotspot")).font(.subheadline.bold())
            Text("\(Int(snapshot.hotspot.rounded())) °C").font(.system(size: 34, weight: .bold).monospacedDigit()).foregroundStyle(color)
            let t = snapshot.trendCPerMin
            Label(String(format: "%+.1f °C/min", t), systemImage: t > 1 ? "arrow.up.right" : t < -1 ? "arrow.down.right" : "arrow.right")
                .font(.caption.monospacedDigit())
                .foregroundStyle(t > 5 ? .orange : .secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(color.opacity(0.1), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(color.opacity(0.6), lineWidth: 1.5))
    }
}

// MARK: - Ventilateurs

struct FansPanel: View {
    let snapshot: Snapshot
    var body: some View {
        Card(title: tr("Ventilateurs", "Fans"), icon: "fan") {
            ForEach(snapshot.fans) { f in
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        SpinningFan(rpm: f.rpm)
                        Text(f.name).font(.subheadline.bold())
                        Text(f.manual ? tr("FORCÉ", "FORCED") : (f.rpm < 1 ? tr("ARRÊTÉ (macOS)", "STOPPED (macOS)") : "AUTO")).font(.caption2.bold())
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .background(f.manual ? Color.accentColor.opacity(0.2) : Color.gray.opacity(0.2), in: Capsule())
                        Spacer()
                        Text("\(Int(f.rpm)) RPM").font(.title3.bold().monospacedDigit())
                    }
                    ProgressView(value: f.percent, total: 100)
                        .tint(f.percent > 80 ? .orange : .accentColor)
                    HStack {
                        Text("min \(Int(f.min))")
                        Spacer()
                        Text(tr("consigne \(Int(f.target)) · \(Int(f.percent)) %", "target \(Int(f.target)) · \(Int(f.percent)) %"))
                        Spacer()
                        Text("max \(Int(f.max))")
                    }
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                .padding(.vertical, 4)
            }
        }
    }
}

/// Icône statique : une animation continue coûtait ~40 % de CPU (mauvais pour la batterie).
struct SpinningFan: View {
    let rpm: Double
    var body: some View {
        Image(systemName: rpm < 1 ? "fan" : "fan.fill").foregroundStyle(.tint)
    }
}

// MARK: - Contrôle

struct ControlPanel: View {
    @Environment(EngineClient.self) private var client
    let snapshot: Snapshot
    @State private var tab = "auto"
    @State private var percent = 50.0
    @State private var preset: CurvePreset = .balanced

    var body: some View {
        Card(title: tr("Contrôle", "Control"), icon: "slider.horizontal.3") {
            Picker("", selection: $tab) {
                Text("Auto (macOS)").tag("auto")
                Text(tr("Manuel", "Manual")).tag("manual")
                Text(tr("Courbe", "Curve")).tag("curve")
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            switch tab {
            case "manual":
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(tr("Vitesse", "Speed"))
                        Slider(value: $percent, in: 0...100, step: 5)
                        Text("\(Int(percent)) %").monospacedDigit().frame(width: 44, alignment: .trailing)
                    }
                    Text(snapshot.fans.map { "\($0.name) : \(Int($0.min + ($0.max - $0.min) * percent / 100)) RPM" }.joined(separator: "  ·  "))
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    HStack {
                        ForEach([0.0, 25, 50, 75, 100], id: \.self) { p in
                            Button("\(Int(p)) %") { percent = p }.controlSize(.small)
                        }
                    }
                }
            case "curve":
                Picker(tr("Profil", "Profile"), selection: $preset) {
                    ForEach(CurvePreset.allCases) { Text($0.label).tag($0) }
                }
                CurveChart(preset: preset, current: snapshot.hotspot)
                    .frame(height: 120)
            default:
                Text(tr("macOS (thermalmonitord) pilote les ventilateurs. Sur Apple Silicon, il privilégie le silence et laisse la puce chauffer avant d'accélérer.", "macOS (thermalmonitord) drives the fans. On Apple Silicon it favors silence and lets the chip heat up before spinning up."))
                    .font(.callout).foregroundStyle(.secondary)
            }

            HStack {
                Text(tr("Actif : \(describe(snapshot.mode))", "Active: \(describe(snapshot.mode))")).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(tr("Appliquer", "Apply")) {
                    switch tab {
                    case "manual": client.setMode(.manual(percent))
                    case "curve": client.setMode(.curve(preset))
                    default: client.setMode(.auto)
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(!snapshot.privileged && tab != "auto")
            }
            Text(tr("Quand vous quittez ThermoPilot, macOS reprend le contrôle des ventilateurs (après 5 s).", "When you quit ThermoPilot, macOS takes back control of the fans (after 5 s)."))
                .font(.caption2).foregroundStyle(.secondary)
            if !snapshot.privileged {
                Label(tr("Le contrôle nécessite le helper root (installé une seule fois, mot de passe admin).", "Control requires the root helper (installed once, admin password)."), systemImage: "info.circle")
                    .font(.caption).foregroundStyle(.orange)
            }
        }
        .onAppear(perform: sync)
        .onChange(of: snapshot.mode) { sync() }
    }

    /// Aligne l'UI sur le mode réellement actif dans le moteur.
    private func sync() {
        let m = snapshot.mode
        tab = m.kind
        if let p = m.percent { percent = p }
        if let p = m.preset, let cp = CurvePreset(rawValue: p) { preset = cp }
    }

    private func describe(_ m: ModeDTO) -> String {
        switch m.kind {
        case "manual": tr("Manuel \(Int(m.percent ?? 0)) %", "Manual \(Int(m.percent ?? 0)) %")
        case "curve": tr("Courbe \(CurvePreset(rawValue: m.preset ?? "")?.label ?? "")", "Curve \(CurvePreset(rawValue: m.preset ?? "")?.label ?? "")")
        default: "Auto (macOS)"
        }
    }
}

struct CurveChart: View {
    let preset: CurvePreset
    let current: Double
    var body: some View {
        let pts = [(30.0, preset.points.first!.1)] + preset.points + [(105.0, 100.0)]
        Chart {
            ForEach(Array(pts.enumerated()), id: \.offset) { _, p in
                LineMark(x: .value("°C", p.0), y: .value("%", p.1)).interpolationMethod(.linear)
                AreaMark(x: .value("°C", p.0), y: .value("%", p.1)).foregroundStyle(.tint.opacity(0.15))
            }
            RuleMark(x: .value(tr("Actuel", "Current"), min(current, 105)))
                .foregroundStyle(Thermal.color(current))
                .annotation(position: .top, alignment: .leading) {
                    Text("\(Int(current))°").font(.caption2.bold()).foregroundStyle(Thermal.color(current))
                }
        }
        .chartXScale(domain: 30...105)
        .chartYScale(domain: 0...100)
        .chartXAxisLabel(tr("Température (°C)", "Temperature (°C)"))
        .chartYAxisLabel(tr("Ventilation (%)", "Fan speed (%)"))
    }
}

// MARK: - Historique

struct HistoryCharts: View {
    let history: [HistoryPoint]
    let fans: [Fan]
    @State private var window = 300.0

    /// Points de la fenêtre choisie, sous-échantillonnés à ~150 pour un rendu léger.
    private var points: [HistoryPoint] {
        let cutoff = Date().addingTimeInterval(-window)
        let all = history.filter { $0.date >= cutoff }
        let step = max(1, all.count / 150)
        return stride(from: 0, to: all.count, by: step).map { all[$0] }
    }

    var body: some View {
        let pts = points
        let maxRPM: Double = (fans.map(\.max).max() ?? 6000) + 200
        Card(title: tr("Historique", "History"), icon: "chart.xyaxis.line") {
            Picker(tr("Fenêtre", "Window"), selection: $window) {
                Text("1 min").tag(60.0)
                Text("5 min").tag(300.0)
                Text("10 min").tag(600.0)
            }
            .pickerStyle(.segmented).frame(width: 240)

            HStack(spacing: 20) {
                TemperatureChart(points: pts)
                FanChart(points: pts, maxRPM: maxRPM)
            }
        }
    }
}

private struct TemperatureChart: View {
    let points: [HistoryPoint]
    var body: some View {
        Chart {
            ForEach(points) { (p: HistoryPoint) in
                LineMark(x: .value("t", p.date), y: .value("°C", p.hotspot), series: .value("s", tr("Point chaud", "Hotspot")))
                    .foregroundStyle(by: .value("s", tr("Point chaud", "Hotspot")))
                LineMark(x: .value("t", p.date), y: .value("°C", p.cpu), series: .value("s", "CPU P max"))
                    .foregroundStyle(by: .value("s", "CPU P max"))
                LineMark(x: .value("t", p.date), y: .value("°C", p.gpu), series: .value("s", "GPU max"))
                    .foregroundStyle(by: .value("s", "GPU max"))
            }
        }
        .chartForegroundStyleScale([tr("Point chaud", "Hotspot"): Color.red, "CPU P max": Color.orange, "GPU max": Color.purple])
        .chartYScale(domain: 20.0...110.0)
        .chartYAxisLabel("°C")
        .frame(height: 180)
    }
}

private struct FanChart: View {
    let points: [HistoryPoint]
    let maxRPM: Double
    var body: some View {
        Chart {
            ForEach(points) { (p: HistoryPoint) in
                ForEach(Array(p.fans.enumerated()), id: \.offset) { (i: Int, rpm: Double) in
                    LineMark(x: .value("t", p.date), y: .value("RPM", rpm), series: .value("f", tr("Ventilateur \(i + 1)", "Fan \(i + 1)")))
                        .foregroundStyle(by: .value("f", tr("Ventilateur \(i + 1)", "Fan \(i + 1)")))
                }
            }
        }
        .chartYScale(domain: 0.0...maxRPM)
        .chartYAxisLabel("RPM")
        .frame(height: 180)
    }
}

// MARK: - Autres capteurs

struct OtherSensors: View {
    let snapshot: Snapshot
    private let ids = ["ssd", "battery", "wifi", "palm", "air"]
    private let icons = ["ssd": "internaldrive", "battery": "battery.75percent", "wifi": "wifi", "palm": "hand.raised", "air": "wind"]

    var body: some View {
        Card(title: tr("Autres capteurs", "Other sensors"), icon: "thermometer.variable.and.figure") {
            LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 5), spacing: 10) {
                ForEach(ids.compactMap { snapshot.group($0) }) { g in
                    let c = Thermal.color(g.max, warn: g.warn, crit: g.crit)
                    HStack {
                        Image(systemName: icons[g.id] ?? "thermometer").foregroundStyle(c).frame(width: 22)
                        VStack(alignment: .leading) {
                            Text(g.name).font(.caption).foregroundStyle(.secondary)
                            Text(String(format: "%.1f °C", g.max)).font(.body.bold().monospacedDigit()).foregroundStyle(c)
                        }
                        Spacer()
                    }
                }
            }
            if !snapshot.cpuCores.isEmpty {
                Divider()
                Text(tr("Charge par cœur CPU", "Load per CPU core")).font(.caption).foregroundStyle(.secondary)
                HStack(alignment: .bottom, spacing: 3) {
                    ForEach(Array(snapshot.cpuCores.enumerated()), id: \.offset) { _, v in
                        RoundedRectangle(cornerRadius: 2)
                            .fill(v > 80 ? Color.orange : Color.accentColor)
                            .frame(height: max(2, v / 100 * 40))
                            .frame(maxWidth: .infinity)
                    }
                }
                .frame(height: 40, alignment: .bottom)
            }
        }
    }
}

struct RawSensors: View {
    let sensors: [Sensor]
    @State private var expanded = false
    var body: some View {
        Card(title: tr("Capteurs bruts (\(sensors.count))", "Raw sensors (\(sensors.count))"), icon: "list.bullet.rectangle") {
            DisclosureGroup(expanded ? tr("Masquer", "Hide") : tr("Afficher toutes les clés SMC", "Show all SMC keys"), isExpanded: $expanded) {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 6), alignment: .leading, spacing: 4) {
                    ForEach(sensors) { s in
                        HStack(spacing: 4) {
                            Text(s.key).font(.caption.monospaced()).foregroundStyle(.secondary)
                            Text(String(format: "%.1f°", s.value)).font(.caption.monospacedDigit()).foregroundStyle(Thermal.color(s.value))
                        }
                    }
                }
                .padding(.top, 6)
            }
        }
    }
}
