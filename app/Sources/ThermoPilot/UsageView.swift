import Charts
import SwiftUI

/// Batterie › Utilisation & diagnostic : historique, apps les plus gourmandes, vidange normale ou pas.
struct UsageView: View {
    @Environment(EngineClient.self) private var client

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let err = client.usageError {
                Label(err, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange).font(.callout)
            }
            if let r = client.usage {
                VerdictBanner(verdict: r.verdict)
                HStack(alignment: .top, spacing: 12) {
                    ForEach(r.checks) { CheckCard(check: $0) }
                }
                TipsCard(tips: r.tips, title: tr("Que faire ?", "What to do?"))
                LevelTimeline(points: r.timeline)
                HStack(alignment: .top, spacing: 16) {
                    AppsCard(report: r).frame(maxWidth: .infinity)
                    DaysCard(days: r.days).frame(maxWidth: .infinity)
                }
                HStack(alignment: .top, spacing: 16) {
                    SessionsCard(sessions: r.sessions).frame(maxWidth: .infinity)
                    SleepCard(sleeps: r.sleeps, reasons: r.wakeReasons).frame(maxWidth: .infinity)
                }
                SourcesFooter(report: r)
            } else if client.usageError == nil {
                HStack { ProgressView().controlSize(.small); Text(tr("Analyse du journal d'alimentation…", "Analyzing the power log…")).foregroundStyle(.secondary) }
            }
        }
        .task {
            // Rafraîchi toutes les 30 s tant que la vue est affichée
            while !Task.isCancelled {
                client.fetchUsage()
                try? await Task.sleep(for: .seconds(client.usage == nil ? 3 : 30))
            }
        }
    }
}

// MARK: - Verdict

struct VerdictBanner: View {
    let verdict: UsageCheck
    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: verdict.icon).font(.system(size: 30)).foregroundStyle(verdict.color)
            VStack(alignment: .leading, spacing: 3) {
                Text(verdict.label).font(.title2.bold())
                Text(verdict.detail).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(16)
        .background(verdict.color.opacity(0.12), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(verdict.color.opacity(0.35)))
    }
}

struct CheckCard: View {
    let check: UsageCheck
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(check.label).font(.subheadline.bold())
                Spacer()
                Image(systemName: check.icon).foregroundStyle(check.color)
            }
            Text(check.value).font(.system(size: 26, weight: .semibold).monospacedDigit()).foregroundStyle(check.color)
            Text(check.reference).font(.caption).foregroundStyle(.secondary)
            Text(check.detail).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(.background, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.quaternary))
    }
}

// MARK: - Courbe du niveau de charge

struct LevelTimeline: View {
    let points: [TimelinePoint]
    @State private var range = 1

    private struct Band: Identifiable {
        let id: Int
        let start: Date
        let end: Date
        let kind: String
    }

    var body: some View {
        let from = Date().addingTimeInterval(range == 0 ? -86400 : -7 * 86400)
        let pts = points.filter { Date(timeIntervalSince1970: $0.t) >= from }
        let bands = makeBands(pts)
        Card(title: tr("Niveau de charge", "Charge level"), icon: "chart.xyaxis.line") {
            HStack {
                Picker("", selection: $range) {
                    Text("24 h").tag(0)
                    Text(tr("7 jours", "7 days")).tag(1)
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 160)
                Spacer()
                LegendDot(color: .green, text: tr("Branché", "Plugged in"))
                LegendDot(color: .indigo, text: tr("En veille", "Asleep"))
                LegendDot(color: .orange, text: tr("Sur batterie, éveillé", "On battery, awake"))
            }
            if pts.count < 2 {
                Text(tr("Pas encore de données sur cette période.", "No data for this period yet.")).foregroundStyle(.secondary).frame(height: 180)
            } else {
                Chart {
                    ForEach(bands) { b in
                        RectangleMark(xStart: .value("Début", b.start), xEnd: .value("Fin", b.end), yStart: .value("", 0), yEnd: .value("", 100))
                            .foregroundStyle(b.kind == "ac" ? Color.green.opacity(0.13) : Color.indigo.opacity(0.13))
                    }
                    ForEach(Array(pts.enumerated()), id: \.offset) { _, p in
                        LineMark(x: .value("Heure", Date(timeIntervalSince1970: p.t)), y: .value("Charge", p.pct))
                            .foregroundStyle(.orange)
                            .interpolationMethod(.monotone)
                    }
                }
                .chartYScale(domain: 0...100)
                .chartYAxis { AxisMarks(values: [0, 20, 50, 80, 100]) { v in AxisGridLine(); AxisValueLabel { Text("\(v.as(Int.self) ?? 0) %") } } }
                .frame(height: 200)
            }
        }
    }

    /// Regroupe les intervalles consécutifs « branché » / « en veille » en bandes de fond.
    private func makeBands(_ pts: [TimelinePoint]) -> [Band] {
        var out: [Band] = []
        var cur: (kind: String, start: Double)?
        for (i, p) in pts.enumerated() {
            let kind = p.ac ? "ac" : p.asleep ? "sleep" : "batt"
            if cur?.kind != kind {
                if let c = cur, c.kind != "batt" {
                    out.append(Band(id: out.count, start: Date(timeIntervalSince1970: c.start), end: Date(timeIntervalSince1970: p.t), kind: c.kind))
                }
                cur = (kind, p.t)
            }
            if i == pts.count - 1, let c = cur, c.kind != "batt" {
                out.append(Band(id: out.count, start: Date(timeIntervalSince1970: c.start), end: Date(timeIntervalSince1970: p.t), kind: c.kind))
            }
        }
        return out
    }
}

struct LegendDot: View {
    let color: Color
    let text: String
    var body: some View {
        HStack(spacing: 4) {
            RoundedRectangle(cornerRadius: 2).fill(color.opacity(0.6)).frame(width: 10, height: 10)
            Text(text).font(.caption).foregroundStyle(.secondary)
        }
    }
}

// MARK: - Apps

struct AppsCard: View {
    let report: UsageReport
    @State private var range = 0

    var body: some View {
        let apps = range == 0 ? report.apps24H : report.apps7D
        let total = range == 0 ? report.total24HWh : report.total7DWh
        let other = range == 0 ? report.other24HWh : report.other7DWh
        let hours = range == 0 ? report.batteryHours24H : report.batteryHours7D
        let denom = max(total, apps.reduce(0) { $0 + $1.wh }, 0.01)
        Card(title: tr("Apps qui vident la batterie", "Apps draining the battery"), icon: "app.badge") {
            HStack {
                Picker("", selection: $range) {
                    Text("24 h").tag(0)
                    Text(tr("7 jours", "7 days")).tag(1)
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 160)
                Spacer()
                if total > 0 {
                    Text(String(format: tr("%.1f Wh sur %@ de batterie", "%.1f Wh over %@ on battery"), total, Fmt.hours(hours))).font(.caption).foregroundStyle(.secondary)
                }
            }
            if apps.isEmpty {
                Text(tr("Aucune mesure pour l'instant. ThermoPilot enregistre la consommation de chaque app quand le Mac fonctionne sur batterie, même app fermée (helper root).", "No measurements yet. ThermoPilot records each app's energy while the Mac runs on battery, even with the app closed (root helper)."))
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else {
                ForEach(apps.prefix(10)) { a in
                    AppRow(name: a.name, wh: a.wh, share: a.sharePct, detail: String(format: tr("%.2f W moy. · %.0f min CPU", "%.2f W avg · %.0f CPU min"), a.avgW, a.cpuMin), color: a.sharePct >= 35 ? .red : a.sharePct >= 12 ? .orange : .blue)
                }
                if other > 0.05 {
                    Divider()
                    AppRow(name: tr("Écran, Wi-Fi et reste du système", "Display, Wi-Fi and rest of the system"), wh: other, share: other / denom * 100, detail: tr("non attribuable à une app", "not attributable to an app"), color: .gray)
                }
            }
        }
    }
}

struct AppRow: View {
    let name: String
    let wh: Double
    let share: Double
    let detail: String
    let color: Color
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(name).font(.callout.weight(.medium)).lineLimit(1)
                Spacer()
                Text(String(format: "%.0f %%", share)).font(.callout.bold().monospacedDigit())
                Text(String(format: "%.2f Wh", wh)).font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 64, alignment: .trailing)
            }
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(.quaternary)
                    Capsule().fill(color.gradient).frame(width: max(3, g.size.width * min(share, 100) / 100))
                }
            }
            .frame(height: 6)
            Text(detail).font(.caption2).foregroundStyle(.secondary)
        }
    }
}

// MARK: - Jours

struct DaysCard: View {
    let days: [DaySummary]
    var body: some View {
        Card(title: tr("Utilisation par jour", "Usage per day"), icon: "calendar") {
            if days.isEmpty {
                Text(tr("Journal d'alimentation vide.", "Power log is empty.")).foregroundStyle(.secondary)
            } else {
                Grid(alignment: .trailing, horizontalSpacing: 12, verticalSpacing: 8) {
                    GridRow {
                        Text(tr("Jour", "Day")).gridColumnAlignment(.leading)
                        Text(tr("Éveillé (batt.)", "Awake (batt.)"))
                        Text(tr("Veille (batt.)", "Asleep (batt.)"))
                        Text(tr("Branché", "Plugged in"))
                        Text(tr("Réveils", "Wake-ups"))
                    }
                    .font(.caption.bold()).foregroundStyle(.secondary)
                    ForEach(days) { d in
                        GridRow {
                            Text(d.label).gridColumnAlignment(.leading)
                            cell(d.awakeBattH, "−\(Int(d.awakeBattPct)) %", rate: d.awakeBattH >= 0.25 ? d.awakeBattPct / d.awakeBattH : nil, good: 8, bad: 15)
                            cell(d.sleepBattH, "−\(Int(d.sleepBattPct)) %", rate: d.sleepBattH >= 1 ? d.sleepBattPct / d.sleepBattH : nil, good: 0.5, bad: 1.5)
                            cell(d.acH, "+\(Int(d.chargedPct)) %", rate: nil, good: 0, bad: 0)
                            Text("\(d.darkwakes)").monospacedDigit().foregroundStyle(.secondary)
                        }
                        .font(.callout)
                    }
                }
                Text(tr("Le rythme (%/h) est coloré : vert = normal, orange = élevé, rouge = anormal.", "The rate (%/h) is color-coded: green = normal, orange = high, red = abnormal.")).font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func cell(_ h: Double, _ pct: String, rate: Double?, good: Double, bad: Double) -> some View {
        if h < 0.02 {
            Text("—").foregroundStyle(.tertiary)
        } else {
            VStack(alignment: .trailing, spacing: 0) {
                Text("\(Fmt.hours(h)) · \(pct)").monospacedDigit()
                if let rate {
                    Text(String(format: rate < 1 ? "%.2f %%/h" : "%.1f %%/h", rate))
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(rate <= good ? .green : rate <= bad ? .orange : .red)
                }
            }
        }
    }
}

// MARK: - Sessions sur batterie

struct SessionsCard: View {
    let sessions: [UsageSession]
    var body: some View {
        Card(title: tr("Sessions sur batterie", "Battery sessions"), icon: "battery.50percent") {
            if sessions.isEmpty {
                Text(tr("Aucune session sur batterie ces 7 derniers jours.", "No battery sessions in the last 7 days.")).foregroundStyle(.secondary)
            }
            ForEach(sessions.prefix(8)) { s in
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(Fmt.dateTime(s.start)) → \(s.ongoing ? tr("maintenant", "now") : Fmt.time(s.end))").font(.callout.weight(.medium))
                        Text(tr("\(Fmt.hours(s.awakeH)) éveillé · \(Fmt.hours(s.sleepH)) en veille", "\(Fmt.hours(s.awakeH)) awake · \(Fmt.hours(s.sleepH)) asleep")).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 2) {
                        Text("\(Int(s.pctStart)) % → \(Int(s.pctEnd)) %").font(.callout.monospacedDigit())
                        if let r = s.awakePctPerH {
                            Text(String(format: tr("%.1f %%/h éveillé", "%.1f %%/h awake"), r)).font(.caption.monospacedDigit())
                                .foregroundStyle(r <= 8 ? .green : r <= 15 ? .orange : .red)
                        }
                    }
                }
                Divider()
            }
        }
    }
}

// MARK: - Veille

struct SleepCard: View {
    let sleeps: [SleepSession]
    let reasons: [WakeReason]
    var body: some View {
        Card(title: tr("Veille : la batterie se vide-t-elle ?", "Sleep: is the battery draining?"), icon: "moon.zzz") {
            let list = sleeps.filter(\.onBattery).prefix(6)
            if list.isEmpty {
                Text(tr("Aucune veille sur batterie récente.", "No recent sleep on battery.")).foregroundStyle(.secondary)
            }
            ForEach(Array(list)) { s in
                HStack(alignment: .top) {
                    Circle().fill(color(s.level)).frame(width: 8, height: 8).padding(.top, 5)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(Fmt.dateTime(s.start)) · \(Fmt.hours(s.hours))").font(.callout.weight(.medium))
                        Text(tr("\(s.darkwakes) réveils", "\(s.darkwakes) wake-ups") + (s.mainReason.map { tr(" · surtout : ", " · mostly: ") + $0 } ?? "")).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 2) {
                        Text("−\(Int(max(0, s.pctStart - s.pctEnd))) %").font(.callout.monospacedDigit())
                        Text(String(format: "%.2f %%/h", s.pctPerH)).font(.caption.monospacedDigit()).foregroundStyle(color(s.level))
                    }
                }
            }
            if !reasons.isEmpty {
                Divider()
                Text(tr("Ce qui réveille le Mac (7 jours)", "What wakes the Mac (7 days)")).font(.subheadline.bold())
                let maxC = Double(reasons.map(\.count).max() ?? 1)
                ForEach(reasons.prefix(5)) { r in
                    HStack {
                        Text(r.label).font(.caption).frame(width: 230, alignment: .leading).lineLimit(1)
                        GeometryReader { g in
                            Capsule().fill(.indigo.gradient).frame(width: max(3, g.size.width * Double(r.count) / maxC))
                        }
                        .frame(height: 6)
                        Text("\(r.count)").font(.caption.monospacedDigit()).frame(width: 36, alignment: .trailing)
                    }
                }
                Text(tr("Ces réveils en arrière-plan sont normaux (quelques secondes chacun) tant que la vidange reste ≤ 0,5 %/h.", "These background wake-ups are normal (a few seconds each) as long as the drain stays ≤ 0.5 %/h."))
                    .font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    private func color(_ level: String) -> Color {
        switch level {
        case "good": .green
        case "warn": .orange
        case "bad": .red
        default: .secondary
        }
    }
}

// MARK: - Sources

struct SourcesFooter: View {
    let report: UsageReport
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if let l = report.logSince {
                Text(tr("Historique veille / charge : journal d'alimentation macOS depuis \(Fmt.dateTime(l)).", "Sleep / charge history: macOS power log since \(Fmt.dateTime(l))."))
            }
            if let r = report.recordingSince {
                Text(tr("Consommation par app et au repos : mesurée par ThermoPilot depuis \(Fmt.dateTime(r)) (conservée 7 jours).", "Per-app and idle power: measured by ThermoPilot since \(Fmt.dateTime(r)) (kept 7 days)."))
            } else {
                Text(tr("Consommation par app : mesurée par ThermoPilot à partir de maintenant (conservée 7 jours).", "Per-app power: measured by ThermoPilot from now on (kept 7 days)."))
            }
        }
        .font(.caption).foregroundStyle(.secondary)
    }
}
