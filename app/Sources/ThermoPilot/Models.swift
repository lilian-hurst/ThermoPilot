import Foundation
import SwiftUI

struct ModeDTO: Codable, Equatable {
    var kind: String          // auto | manual | curve
    var percent: Double?
    var preset: String?

    static let auto = ModeDTO(kind: "auto")
    static func manual(_ p: Double) -> ModeDTO { ModeDTO(kind: "manual", percent: p.rounded()) }
    static func curve(_ p: CurvePreset) -> ModeDTO { ModeDTO(kind: "curve", preset: p.rawValue) }
}

struct Fan: Decodable, Identifiable {
    let id: Int
    let name: String
    let rpm: Double
    let min: Double
    let max: Double
    let target: Double
    let manual: Bool
    let percent: Double
}

struct SensorGroup: Decodable, Identifiable {
    let id: String
    let name: String
    let avg: Double
    let max: Double
    let count: Int
    let warn: Double
    let crit: Double
}

struct Sensor: Decodable, Identifiable {
    let key: String
    let group: String
    let value: Double
    var id: String { key }
}

struct Advice: Decodable {
    let level: String         // cool | normal | warm | hot | critical
    let action: String        // none | increase | decrease | max
    let title: String
    let message: String
    let suggestedPercent: Double
    let suggestedRpm: [Double]
}

struct Snapshot: Decodable {
    let timestamp: Double
    let privileged: Bool
    let cpuModel: String
    let hwModel: String
    let mode: ModeDTO
    let fans: [Fan]
    let groups: [SensorGroup]
    let sensors: [Sensor]
    let cpuUsage: Double?
    let cpuCores: [Double]
    let gpuUsage: Double?
    let powerW: Double?
    let hotspot: Double
    let trendCPerMin: Double
    let safetyOverride: Bool
    let advice: Advice
    let lastError: String?
    let battery: BatteryInfo?
    let charge: ChargeStatus
    let powerSettings: PmsetProfiles?
    let pmsetBackup: [String: Int]?
    let topProcesses: [ProcEnergy]
    let batteryTips: [Tip]
    let conflicts: [String]

    func group(_ id: String) -> SensorGroup? { groups.first { $0.id == id } }
}

struct HistoryPoint: Identifiable {
    let id = UUID()
    let date: Date
    let hotspot: Double
    let cpu: Double
    let gpu: Double
    let fans: [Double]
    let cpuUsage: Double
    let gpuUsage: Double
    let batteryPct: Double?
    let batteryPowerW: Double?
}

struct BatteryInfo: Decodable {
    let percent: Double
    let currentMah: Double
    let maxMah: Double
    let designMah: Double
    let healthPct: Double
    let rawHealthPct: Double?
    let nominalMah: Double?
    let cycles: Int
    let designCycles: Int
    let temperatureC: Double
    let voltageV: Double
    let amperageA: Double
    let powerW: Double
    let isCharging: Bool
    let externalConnected: Bool
    let fullyCharged: Bool
    let minutesRemaining: Double?
    let adapterWatts: Double?
    let adapterName: String?
    let systemPowerInW: Double?
}

struct ChargeConfig: Decodable, Equatable {
    var enabled: Bool
    var limit: Int
    var heatProtection: Bool
    var dischargeToLimit: Bool

    var json: [String: Any] {
        ["enabled": enabled, "limit": limit, "heat_protection": heatProtection, "discharge_to_limit": dischargeToLimit]
    }
}

struct ChargeStatus: Decodable {
    let config: ChargeConfig
    let inhibitKey: String?
    let adapterKey: String?
    let method: String
    let chargingInhibited: Bool
    let adapterDisabled: Bool
    let heatPaused: Bool
    let state: String
    let effective: Bool?
    let lastError: String?
}

struct PmsetProfiles: Decodable {
    let battery: [String: Int]
    let ac: [String: Int]
}

struct ProcEnergy: Decodable, Identifiable {
    let name: String
    let pid: Int32
    let count: Int
    let watts: Double
    let cpuPct: Double
    var id: String { name }
}

struct Tip: Decodable, Identifiable {
    let id: String
    let level: String     // good | info | warn | bad
    let title: String
    let message: String
    let action: String?
    let actionLabel: String?

    var color: Color {
        switch level {
        case "good": .green
        case "warn": .orange
        case "bad": .red
        default: .blue
        }
    }
    var icon: String {
        switch level {
        case "good": "checkmark.circle.fill"
        case "warn": "exclamationmark.triangle.fill"
        case "bad": "xmark.octagon.fill"
        default: "lightbulb.fill"
        }
    }
}

enum Fmt {
    static func minutes(_ m: Double?) -> String {
        guard let m, m > 0 else { return "—" }
        let h = Int(m) / 60, mm = Int(m) % 60
        return h > 0 ? "\(h) h \(String(format: "%02d", mm))" : "\(mm) min"
    }
}

enum CurvePreset: String, CaseIterable, Identifiable {
    case silent, balanced, performance
    var id: String { rawValue }
    var label: String {
        switch self {
        case .silent: tr("Silencieux", "Silent")
        case .balanced: tr("Équilibré", "Balanced")
        case .performance: "Performance"
        }
    }
    /// Miroir des courbes du moteur Rust (°C, % entre min et max)
    var points: [(Double, Double)] {
        switch self {
        case .silent: [(70, 0), (85, 35), (95, 100)]
        case .balanced: [(55, 0), (70, 25), (82, 60), (92, 100)]
        case .performance: [(45, 15), (60, 40), (75, 80), (85, 100)]
        }
    }
}

enum Thermal {
    static func color(_ t: Double, warn: Double = 85, crit: Double = 98) -> Color {
        if t < warn - 20 { return .green }
        if t < warn - 8 { return .mint }
        if t < warn { return .yellow }
        if t < crit - 4 { return .orange }
        return .red
    }

    static func levelColor(_ level: String) -> Color {
        switch level {
        case "cool": .green
        case "normal": .mint
        case "warm": .yellow
        case "hot": .orange
        default: .red
        }
    }

    static func levelIcon(_ level: String) -> String {
        switch level {
        case "cool": "snowflake"
        case "normal": "checkmark.seal.fill"
        case "warm": "thermometer.medium"
        case "hot": "flame.fill"
        default: "exclamationmark.triangle.fill"
        }
    }
}

// MARK: - Utilisation de la batterie (requête "usage")

struct UsageCheck: Decodable, Identifiable {
    let id: String
    let label: String
    let value: String
    let reference: String
    let detail: String
    let level: String  // good | info | warn | bad | unknown

    var color: Color {
        switch level {
        case "good": .green
        case "info": .blue
        case "warn": .orange
        case "bad": .red
        default: .secondary
        }
    }
    var icon: String {
        switch level {
        case "good": "checkmark.seal.fill"
        case "info": "info.circle.fill"
        case "warn": "exclamationmark.triangle.fill"
        case "bad": "xmark.octagon.fill"
        default: "hourglass"
        }
    }
}

struct DaySummary: Decodable, Identifiable {
    let date: String
    let label: String
    let awakeBattH: Double
    let awakeBattPct: Double
    let sleepBattH: Double
    let sleepBattPct: Double
    let acH: Double
    let chargedPct: Double
    let darkwakes: Int
    var id: String { date }
}

struct UsageSession: Decodable, Identifiable {
    let start: Double
    let end: Double
    let pctStart: Double
    let pctEnd: Double
    let awakeH: Double
    let sleepH: Double
    let awakePctPerH: Double?
    let ongoing: Bool
    var id: Double { start }
}

struct SleepSession: Decodable, Identifiable {
    let start: Double
    let end: Double
    let pctStart: Double
    let pctEnd: Double
    let hours: Double
    let pctPerH: Double
    let darkwakes: Int
    let onBattery: Bool
    let mainReason: String?
    let level: String
    var id: Double { start }
}

struct AppUsage: Decodable, Identifiable {
    let name: String
    let wh: Double
    let sharePct: Double
    let cpuMin: Double
    let avgW: Double
    var id: String { name }
}

struct WakeReason: Decodable, Identifiable {
    let label: String
    let count: Int
    var id: String { label }
}

struct SleepBlocker: Decodable, Identifiable {
    let pid: Int32
    let name: String
    let kind: String
    let since: String
    let label: String
    var id: String { "\(pid)\(kind)" }
}

struct TimelinePoint: Decodable {
    let t: Double
    let pct: Double
    let ac: Bool
    let asleep: Bool
}

struct UsageReport: Decodable {
    let generatedAt: Double
    let recordingSince: Double?
    let logSince: Double?
    let verdict: UsageCheck
    let checks: [UsageCheck]
    let days: [DaySummary]
    let sessions: [UsageSession]
    let sleeps: [SleepSession]
    let wakeReasons: [WakeReason]
    let apps24H: [AppUsage]
    let apps7D: [AppUsage]
    let total24HWh: Double
    let total7DWh: Double
    let other24HWh: Double
    let other7DWh: Double
    let batteryHours24H: Double
    let batteryHours7D: Double
    let blockers: [SleepBlocker]
    let timeline: [TimelinePoint]
    let tips: [Tip]
}

extension Fmt {
    static func hours(_ h: Double) -> String {
        let m = Int((h * 60).rounded())
        return m >= 60 ? "\(m / 60) h \(String(format: "%02d", m % 60))" : "\(m) min"
    }
    static func dateTime(_ t: Double) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: Lang.english ? "en_US" : "fr_FR")
        f.dateFormat = "EEE d, HH:mm"
        return f.string(from: Date(timeIntervalSince1970: t))
    }
    static func time(_ t: Double) -> String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f.string(from: Date(timeIntervalSince1970: t))
    }
}

// MARK: - Langue de l'interface

/// Langue choisie : "system" (par défaut), "fr" ou "en". Stockée dans les préférences.
enum Lang {
    static let key = "appLanguage"
    static let choices: [(id: String, label: String)] = [("system", "Système / System"), ("fr", "Français"), ("en", "English")]

    /// Langue effective (résout « système » avec la langue préférée de macOS)
    static private(set) var english: Bool = resolve(UserDefaults.standard.string(forKey: key) ?? "system")

    static var code: String { english ? "en" : "fr" }

    static func resolve(_ choice: String) -> Bool {
        switch choice {
        case "en": return true
        case "fr": return false
        default: return !(Locale.preferredLanguages.first ?? "en").hasPrefix("fr")
        }
    }

    static func set(_ choice: String) {
        UserDefaults.standard.set(choice, forKey: key)
        english = resolve(choice)
    }
}

/// Texte de l'interface dans la langue choisie.
func tr(_ fr: String, _ en: String) -> String { Lang.english ? en : fr }
