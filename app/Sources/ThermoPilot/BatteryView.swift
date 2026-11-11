import Charts
import SwiftUI

struct BatteryView: View {
    @Environment(EngineClient.self) private var client
    @AppStorage("batteryPage") private var page = 0

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HeaderView()
                Picker("", selection: $page) {
                    Text("Vue d'ensemble").tag(0)
                    Text("Utilisation & diagnostic").tag(1)
                }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 380)
                if page == 1 {
                    UsageView()
                } else if let s = client.snapshot, let b = s.battery {
                    if let err = client.commandError ?? s.charge.lastError {
                        Label(err, systemImage: "xmark.octagon.fill").foregroundStyle(.red).font(.callout)
                    }
                    HStack(spacing: 12) {
                        ChargeRing(battery: b, charge: s.charge)
                        PowerCard(battery: b)
                        HealthCard(battery: b)
                    }
                    TipsCard(tips: s.batteryTips)
                    HStack(alignment: .top, spacing: 16) {
                        ChargeLimitCard(snapshot: s).frame(maxWidth: .infinity)
                        PowerSettingsCard(snapshot: s).frame(maxWidth: .infinity)
                    }
                    HStack(alignment: .top, spacing: 16) {
                        ConsumersCard(processes: s.topProcesses, onBattery: !b.externalConnected).frame(maxWidth: .infinity)
                        BatteryHistory(history: client.history).frame(maxWidth: .infinity)
                    }
                } else if client.snapshot != nil {
                    Text("Aucune batterie détectée.").foregroundStyle(.secondary)
                } else {
                    ConnectingView()
                }
            }
            .padding(20)
        }
        .background(.background.secondary)
    }
}

// MARK: - Cartes du haut

struct ChargeRing: View {
    let battery: BatteryInfo
    let charge: ChargeStatus
    var body: some View {
        let b = battery
        let color: Color = b.percent <= 20 ? .red : b.percent <= 40 ? .orange : .green
        HStack(spacing: 16) {
            ZStack {
                Circle().stroke(.quaternary, lineWidth: 10)
                Circle().trim(from: 0, to: b.percent / 100)
                    .stroke(color, style: StrokeStyle(lineWidth: 10, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                if charge.config.enabled {
                    Circle().trim(from: Double(charge.config.limit) / 100 - 0.004, to: Double(charge.config.limit) / 100 + 0.004)
                        .stroke(.primary, lineWidth: 14).rotationEffect(.degrees(-90))
                }
                VStack(spacing: 0) {
                    Text("\(Int(b.percent)) %").font(.title.bold().monospacedDigit())
                    Image(systemName: b.isCharging ? "bolt.fill" : b.externalConnected ? "powerplug.fill" : "battery.75percent")
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: 110, height: 110)
            VStack(alignment: .leading, spacing: 6) {
                Text(charge.state).font(.headline)
                if let m = b.minutesRemaining {
                    Label(b.isCharging ? "Pleine charge dans \(Fmt.minutes(m))" : "Autonomie \(Fmt.minutes(m))", systemImage: "clock")
                        .font(.callout).foregroundStyle(.secondary)
                }
                if charge.config.enabled {
                    Label("Limite \(charge.config.limit) %", systemImage: "gauge.with.dots.needle.67percent").font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(16)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.quaternary))
    }
}

struct PowerCard: View {
    let battery: BatteryInfo
    var body: some View {
        let b = battery
        Card(title: "Énergie", icon: "bolt") {
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 6) {
                GridRow {
                    Text("Batterie").foregroundStyle(.secondary)
                    Text(b.powerW == 0 ? "au repos" : String(format: "%@%.1f W", b.powerW > 0 ? "+" : "", b.powerW))
                        .bold().monospacedDigit().foregroundStyle(b.powerW < -20 ? .orange : .primary)
                }
                if let w = b.systemPowerInW {
                    GridRow { Text("Système (secteur)").foregroundStyle(.secondary); Text(String(format: "%.1f W", w)).bold().monospacedDigit() }
                }
                if let a = b.adapterWatts {
                    GridRow { Text("Adaptateur").foregroundStyle(.secondary); Text("\(Int(a)) W \(b.adapterName ?? "")").monospacedDigit() }
                }
                GridRow { Text("Tension / courant").foregroundStyle(.secondary); Text(String(format: "%.2f V · %.2f A", b.voltageV, b.amperageA)).monospacedDigit() }
            }
            .font(.callout)
        }
    }
}

struct HealthCard: View {
    let battery: BatteryInfo
    var body: some View {
        let b = battery
        let hColor: Color = b.healthPct >= 90 ? .green : b.healthPct >= 80 ? .yellow : .red
        let tColor: Color = b.temperatureC >= 40 ? .red : b.temperatureC >= 35 ? .orange : .green
        Card(title: "Santé", icon: "heart.text.square") {
            HStack(alignment: .firstTextBaseline) {
                Text(String(format: "%.0f %%", b.healthPct)).font(.title.bold()).foregroundStyle(hColor)
                VStack(alignment: .leading, spacing: 1) {
                    Text("\(Int(b.nominalMah ?? b.maxMah)) / \(Int(b.designMah)) mAh").font(.caption).foregroundStyle(.secondary)
                    if let raw = b.rawHealthPct {
                        Text(String(format: "jauge brute : %.1f %% (%d mAh)", raw, Int(b.maxMah)))
                            .font(.caption2).foregroundStyle(.tertiary)
                            .help("Réglages macOS affiche la capacité nominale (lissée). La valeur brute est la mesure instantanée de la jauge, plus pessimiste et plus variable.")
                    }
                }
                Spacer()
                Label(String(format: "%.1f °C", b.temperatureC), systemImage: "thermometer.medium").foregroundStyle(tColor).monospacedDigit()
            }
            VStack(alignment: .leading, spacing: 3) {
                ProgressView(value: Double(min(b.cycles, b.designCycles)), total: Double(b.designCycles))
                    .tint(Double(b.cycles) / Double(max(b.designCycles, 1)) > 0.8 ? .orange : .accentColor)
                Text("\(b.cycles) cycles sur \(b.designCycles) prévus").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: - Conseils

struct TipsCard: View {
    @Environment(EngineClient.self) private var client
    let tips: [Tip]
    var title = "Conseils autonomie & durée de vie"
    @State private var confirmQuit: Tip?

    var body: some View {
        Card(title: title, icon: "wand.and.stars") {
            if tips.isEmpty {
                Text("Rien à signaler.").foregroundStyle(.secondary)
            }
            ForEach(Array(tips.enumerated()), id: \.offset) { _, t in
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: t.icon).foregroundStyle(t.color).font(.title3).frame(width: 24)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(t.title).font(.subheadline.bold())
                        Text(t.message).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer()
                    if let action = t.action, let label = t.actionLabel {
                        Button(label) {
                            if action.hasPrefix("quit:") { confirmQuit = t } else { client.perform(action) }
                        }
                        .buttonStyle(.borderedProminent).tint(t.color).controlSize(.small)
                    }
                }
                .padding(10)
                .background(t.color.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
            }
        }
        .confirmationDialog(confirmQuit.map { "Quitter « \($0.action?.split(separator: ":", maxSplits: 2).last.map(String.init) ?? "") » ?" } ?? "",
                            isPresented: Binding(get: { confirmQuit != nil }, set: { if !$0 { confirmQuit = nil } })) {
            Button("Quitter l'application", role: .destructive) {
                if let a = confirmQuit?.action { client.perform(a) }
                confirmQuit = nil
            }
        } message: {
            Text("L'application pourra vous demander d'enregistrer vos documents.")
        }
    }
}

// MARK: - Limiteur de charge

struct ChargeLimitCard: View {
    @Environment(EngineClient.self) private var client
    let snapshot: Snapshot
    @State private var cfg = ChargeConfig(enabled: false, limit: 80, heatProtection: true, dischargeToLimit: false)

    var body: some View {
        let c = snapshot.charge
        Card(title: "Limiteur de charge", icon: "battery.100percent.bolt") {
            Toggle("Limiter la charge", isOn: $cfg.enabled)
            HStack {
                Slider(value: Binding(get: { Double(cfg.limit) }, set: { cfg.limit = Int($0) }), in: 50...100, step: 5)
                Text("\(cfg.limit) %").monospacedDigit().frame(width: 48, alignment: .trailing)
            }
            .disabled(!cfg.enabled)
            Toggle("Protection thermique (pause au-delà de 35 °C)", isOn: $cfg.heatProtection)
            if c.adapterKey != nil {
                Toggle("Redescendre à la limite si la batterie est au-dessus", isOn: $cfg.dischargeToLimit)
                    .disabled(!cfg.enabled)
            }
            Text(methodText(c)).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if c.effective == false {
                Label("Le firmware ne semble pas appliquer la commande.", systemImage: "exclamationmark.triangle.fill")
                    .font(.caption).foregroundStyle(.red)
            }
            if !snapshot.conflicts.isEmpty {
                Label("Conflit possible : \(snapshot.conflicts.joined(separator: ", "))", systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
            }
            HStack {
                Text("80 % est le meilleur compromis si le Mac reste souvent branché.").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Appliquer") { client.setCharge(cfg) }
                    .buttonStyle(.borderedProminent)
                    .disabled(!snapshot.privileged || c.method == "none" || cfg == c.config)
            }
            if !snapshot.privileged {
                Label("Nécessite le helper root.", systemImage: "lock").font(.caption).foregroundStyle(.orange)
            }
        }
        .onAppear { cfg = c.config }
        .onChange(of: c.config) { cfg = c.config }
    }

    private func methodText(_ c: ChargeStatus) -> String {
        switch c.method {
        case "inhibit": "Méthode : pause de charge via la clé SMC \(c.inhibitKey ?? "")."
        case "sailing": "Méthode « navigation » : ce firmware n'a pas de clé de pause, l'adaptateur (\(c.adapterKey ?? "")) est coupé à la limite puis réactivé 5 points plus bas. Réactivé automatiquement avant la veille."
        default: "Ce firmware n'expose aucune clé de contrôle de charge. Utilisez la « Recharge optimisée » de macOS."
        }
    }
}

// MARK: - Réglages macOS

struct PowerSettingsCard: View {
    @Environment(EngineClient.self) private var client
    let snapshot: Snapshot

    var body: some View {
        Card(title: "Réglages d'alimentation macOS", icon: "gearshape.2") {
            if let p = snapshot.powerSettings {
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
                    GridRow {
                        Text("").gridColumnAlignment(.leading)
                        Text("Sur batterie").font(.caption.bold()).foregroundStyle(.secondary)
                        Text("Sur secteur").font(.caption.bold()).foregroundStyle(.secondary)
                    }
                    if p.battery["powermode"] != nil {
                        GridRow {
                            Text("Mode d'énergie")
                            modePicker("battery", p.battery["powermode"] ?? 0)
                            modePicker("ac", p.ac["powermode"] ?? 0)
                        }
                    } else if p.battery["lowpowermode"] != nil {
                        GridRow {
                            Text("Économie d'énergie")
                            toggle("battery", "lowpowermode", p.battery)
                            toggle("ac", "lowpowermode", p.ac)
                        }
                    }
                    GridRow {
                        Text("Extinction écran")
                        minutesPicker("battery", "displaysleep", p.battery["displaysleep"] ?? 0, [2, 5, 10, 15, 30])
                        minutesPicker("ac", "displaysleep", p.ac["displaysleep"] ?? 0, [5, 10, 15, 30, 60])
                    }
                    GridRow {
                        Text("Mise en veille")
                        minutesPicker("battery", "sleep", p.battery["sleep"] ?? 0, [1, 5, 10, 15, 30])
                        minutesPicker("ac", "sleep", p.ac["sleep"] ?? 0, [10, 30, 60, 120])
                    }
                    GridRow {
                        Text("Power Nap")
                        toggle("battery", "powernap", p.battery)
                        toggle("ac", "powernap", p.ac)
                    }
                }
                .disabled(!snapshot.privileged)
                Divider()
                HStack {
                    Button {
                        client.powerPreset("battery_saver")
                    } label: { Label("Optimiser l'autonomie", systemImage: "leaf.fill") }
                        .buttonStyle(.borderedProminent).tint(.green)
                    if snapshot.pmsetBackup != nil {
                        Button("Restaurer mes réglages") { client.powerPreset("restore") }
                    }
                }
                .disabled(!snapshot.privileged)
                Text("Profil batterie : économie d'énergie, écran éteint après 5 min, veille après 15 min, Power Nap coupé, luminosité réduite. Vos réglages actuels sont sauvegardés.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else {
                Text("Lecture des réglages pmset…").foregroundStyle(.secondary)
            }
        }
    }

    private func modePicker(_ profile: String, _ value: Int) -> some View {
        Picker("", selection: Binding(get: { value }, set: { client.setPmset(profile, "powermode", $0) })) {
            Text("Automatique").tag(0)
            Text("Économie").tag(1)
            Text("Haute perf.").tag(2)
        }
        .labelsHidden().fixedSize()
    }

    private func minutesPicker(_ profile: String, _ key: String, _ value: Int, _ options: [Int]) -> some View {
        // « Jamais » (0) en dernier
        func rank(_ m: Int) -> Int { m == 0 ? Int.max : m }
        let all: [Int] = Set(options + [value]).sorted { rank($0) < rank($1) }
        let binding = Binding<Int>(get: { value }, set: { client.setPmset(profile, key, $0) })
        return Picker("", selection: binding) {
            ForEach(all, id: \.self) { (m: Int) in
                Text(m == 0 ? "Jamais" : "\(m) min").tag(m)
            }
        }
        .labelsHidden().fixedSize()
    }

    private func toggle(_ profile: String, _ key: String, _ dict: [String: Int]) -> some View {
        Toggle("", isOn: Binding(get: { (dict[key] ?? 0) == 1 }, set: { client.setPmset(profile, key, $0 ? 1 : 0) }))
            .labelsHidden().toggleStyle(.switch).controlSize(.small)
    }
}

// MARK: - Consommateurs

struct ConsumersCard: View {
    @Environment(EngineClient.self) private var client
    let processes: [ProcEnergy]
    let onBattery: Bool

    var body: some View {
        Card(title: "Plus gros consommateurs", icon: "chart.bar.xaxis") {
            if processes.isEmpty {
                Text("Mesure en cours…").foregroundStyle(.secondary)
            }
            let maxW = max(processes.map(\.watts).max() ?? 1, 0.5)
            ForEach(processes.prefix(8)) { p in
                HStack(spacing: 8) {
                    if let app = NSRunningApplication(processIdentifier: p.pid), let icon = app.icon {
                        Image(nsImage: icon).resizable().frame(width: 18, height: 18)
                    } else {
                        Image(systemName: "gearshape").frame(width: 18)
                    }
                    Text(p.name + (p.count > 1 ? " (\(p.count))" : "")).lineLimit(1)
                    Spacer()
                    GeometryReader { g in
                        RoundedRectangle(cornerRadius: 3).fill(p.watts > 3 ? Color.orange : Color.accentColor)
                            .frame(width: max(2, g.size.width * p.watts / maxW))
                    }
                    .frame(width: 80, height: 8)
                    Text(String(format: "%.2f W", p.watts)).monospacedDigit().frame(width: 62, alignment: .trailing)
                    Text(String(format: "%.0f %%", p.cpuPct)).monospacedDigit().foregroundStyle(.secondary).frame(width: 44, alignment: .trailing)
                }
                .font(.callout)
            }
            Text("Énergie mesurée par macOS (CPU + GPU attribués à chaque processus). Les processus d'autres utilisateurs ne sont visibles qu'avec le helper root.")
                .font(.caption2).foregroundStyle(.secondary)
        }
    }
}

struct BatteryHistory: View {
    let allHistory: [HistoryPoint]
    init(history: [HistoryPoint]) { allHistory = history }
    private var history: [HistoryPoint] {
        let step = max(1, allHistory.count / 150)
        return stride(from: 0, to: allHistory.count, by: step).map { allHistory[$0] }
    }
    var body: some View {
        Card(title: "Historique batterie (10 min)", icon: "chart.line.uptrend.xyaxis") {
            Chart(history) { p in
                if let pct = p.batteryPct {
                    LineMark(x: .value("t", p.date), y: .value("Charge", pct)).foregroundStyle(.green)
                }
            }
            .chartYScale(domain: 0...100)
            .chartYAxisLabel("Charge %")
            .frame(height: 80)
            Chart(history) { p in
                if let w = p.batteryPowerW {
                    AreaMark(x: .value("t", p.date), y: .value("W", w))
                        .foregroundStyle(w >= 0 ? Color.green.opacity(0.3) : Color.orange.opacity(0.3))
                    LineMark(x: .value("t", p.date), y: .value("W", w)).foregroundStyle(.orange)
                }
            }
            .chartYAxisLabel("W (+ charge / − décharge)")
            .frame(height: 80)
        }
    }
}
