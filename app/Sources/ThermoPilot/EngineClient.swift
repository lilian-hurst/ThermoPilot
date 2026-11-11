import AppKit
import Foundation
import Observation
import UserNotifications

/// Connexion JSON-lines sur socket Unix vers le moteur Rust.
final class SocketConnection {
    private let fd: Int32
    private var buffer = Data()

    init(path: String) throws {
        fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw EngineError.io("socket()") }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let maxLen = MemoryLayout.size(ofValue: addr.sun_path)
        guard path.utf8.count < maxLen else { close(fd); throw EngineError.io("chemin de socket trop long") }
        withUnsafeMutablePointer(to: &addr.sun_path) {
            $0.withMemoryRebound(to: CChar.self, capacity: maxLen) { _ = strncpy($0, path, maxLen - 1) }
        }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0 else { close(fd); throw EngineError.io("connexion impossible à \(path)") }
        var tv = timeval(tv_sec: 3, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        var one: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
    }

    deinit { close(fd) }

    func request(_ obj: [String: Any]) throws -> Data {
        var line = try JSONSerialization.data(withJSONObject: obj)
        line.append(0x0A)
        let written = line.withUnsafeBytes { write(fd, $0.baseAddress, line.count) }
        guard written == line.count else { throw EngineError.io("écriture socket") }
        var chunk = [UInt8](repeating: 0, count: 65536)
        while true {
            if let nl = buffer.firstIndex(of: 0x0A) {
                let msg = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                return Data(msg)
            }
            let n = read(fd, &chunk, chunk.count)
            guard n > 0 else { throw EngineError.io("connexion fermée") }
            buffer.append(chunk, count: n)
        }
    }
}

enum EngineError: LocalizedError {
    case io(String), engine(String)
    var errorDescription: String? {
        switch self {
        case .io(let s): "E/S : \(s)"
        case .engine(let s): s
        }
    }
}

private struct StatusEnvelope: Decodable { let ok: Bool; let error: String?; let data: Snapshot? }
private struct UsageEnvelope: Decodable { let ok: Bool; let error: String?; let data: UsageReport? }
private struct AckEnvelope: Decodable { let ok: Bool; let error: String? }

enum ConnectionState: Equatable {
    case connecting, root, readOnly, outdated, failed(String)
}

@MainActor
@Observable
final class EngineClient {
    static let rootSocket = ProcessInfo.processInfo.environment["THERMOPILOT_ROOT_SOCKET"] ?? "/var/run/fanengine.sock"
    static let helperPlist = "/Library/LaunchDaemons/com.lilian.fanengine.plist"
    static var userSocket: String { NSTemporaryDirectory() + "fanengine-ui.sock" }

    var snapshot: Snapshot?
    var history: [HistoryPoint] = []
    /// Indicateur de la barre des menus (seul état mis à jour quand aucune fenêtre n'est visible)
    var menuHotspot: Int?
    var menuLevel = "normal"
    var state: ConnectionState = .connecting
    var commandError: String?
    var usage: UsageReport?
    /// Message si le rapport d'utilisation n'est pas disponible
    var usageError: String?
    var notificationsEnabled = true

    @ObservationIgnored private let io = DispatchQueue(label: "thermopilot.io")
    @ObservationIgnored nonisolated(unsafe) private var conn: SocketConnection?
    @ObservationIgnored private var userEngine: Process?
    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored private var lastLevel = "normal"
    @ObservationIgnored private var lastNotified = Date.distantPast
    @ObservationIgnored private var started = false
    @ObservationIgnored private var latest: Snapshot?
    @ObservationIgnored private var historyBuffer: [HistoryPoint] = []

    var helperInstalled: Bool { FileManager.default.fileExists(atPath: Self.helperPlist) }

    func start() {
        guard !started else { return }
        started = true
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
        NotificationCenter.default.addObserver(forName: NSApplication.willTerminateNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.stopUserEngine() }
        }
        for name in [NSWindow.didChangeOcclusionStateNotification, NSApplication.didBecomeActiveNotification,
                     NSWindow.willCloseNotification, NSApplication.didResignActiveNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    self?.flushToUI()
                    // La fermeture n'est effective qu'après la notification
                    DispatchQueue.main.async { self?.scheduleTimer() }
                }
            }
        }
        connect()
        scheduleTimer()
    }

    /// 1 s quand une fenêtre est visible, 5 s sinon (barre des menus seule) : moins de réveils CPU.
    private func scheduleTimer() {
        let interval: TimeInterval = uiVisible ? 1 : 5
        if let t = timer, t.isValid, t.timeInterval == interval { return }
        timer?.invalidate()
        let t = Timer(timeInterval: interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.poll() }
        }
        t.tolerance = interval * 0.2 // regroupement des réveils avec le reste du système
        RunLoop.main.add(t, forMode: .common)
        timer = t
        if interval == 1 { poll() }
    }

    // MARK: Connexion

    func connect() {
        state = .connecting
        let rootPath = Self.rootSocket
        let userPath = Self.userSocket
        io.async { [weak self] in
            if let c = try? SocketConnection(path: rootPath) {
                self?.conn = c
                DispatchQueue.main.async { self?.state = .root; self?.stopUserEngine() }
                return
            }
            DispatchQueue.main.async { self?.launchUserEngine(socket: userPath) }
        }
    }

    private func engineBinary() -> URL? {
        var candidates = [Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/fanengine")]
        if let env = ProcessInfo.processInfo.environment["FANENGINE_PATH"] { candidates.insert(URL(fileURLWithPath: env), at: 0) }
        let exe = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
        candidates.append(exe.appendingPathComponent("fanengine"))
        candidates.append(exe.appendingPathComponent("../../../../engine/target/release/fanengine").standardized)
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    private func launchUserEngine(socket: String) {
        if userEngine?.isRunning != true {
            guard let bin = engineBinary() else {
                state = .failed("Binaire fanengine introuvable dans l'app.")
                return
            }
            let p = Process()
            p.executableURL = bin
            p.arguments = ["serve", "--socket", socket, "--exit-with-parent"]
            p.standardOutput = FileHandle.nullDevice
            do { try p.run() } catch {
                state = .failed("Lancement du moteur impossible : \(error.localizedDescription)")
                return
            }
            userEngine = p
        }
        io.async { [weak self] in
            for _ in 0..<30 {
                if let c = try? SocketConnection(path: socket) {
                    self?.conn = c
                    DispatchQueue.main.async { self?.state = .readOnly }
                    return
                }
                usleep(150_000)
            }
            DispatchQueue.main.async { self?.state = .failed("Le moteur ne répond pas.") }
        }
    }

    private func stopUserEngine() {
        if let p = userEngine, p.isRunning { p.terminate() }
        userEngine = nil
    }

    // MARK: Polling

    private func poll() {
        guard state == .root || state == .readOnly else { return }
        io.async { [weak self] in
            guard let self, let conn = self.conn else { return }
            do {
                let data = try conn.request(["cmd": "status"])
                let env = try JSONDecoder.snake.decode(StatusEnvelope.self, from: data)
                DispatchQueue.main.async {
                    if let snap = env.data { self.ingest(snap) }
                }
            } catch is DecodingError {
                // Le helper root installé parle une ancienne version du protocole
                DispatchQueue.main.async { if self.state == .root { self.state = .outdated } }
            } catch {
                self.conn = nil
                DispatchQueue.main.async { self.connect() }
            }
        }
    }

    /// Vrai si une fenêtre de l'app (tableau de bord ou popover de la barre des menus) est réellement visible.
    private var uiVisible: Bool {
        NSApp.windows.contains { w in
            w.isVisible && w.occlusionState.contains(.visible)
                && !String(describing: type(of: w)).contains("StatusBar")
        }
    }

    /// Publie les dernières données vers l'UI (appelé aussi quand une fenêtre réapparaît).
    func flushToUI() {
        guard let s = latest else { return }
        snapshot = s
        history = historyBuffer
    }

    private func ingest(_ s: Snapshot) {
        latest = s
        let hot = Int(s.hotspot.rounded())
        if menuHotspot != hot { menuHotspot = hot }
        if menuLevel != s.advice.level { menuLevel = s.advice.level }
        let point = HistoryPoint(
            date: Date(timeIntervalSince1970: s.timestamp),
            hotspot: s.hotspot,
            cpu: s.group("cpu_p")?.max ?? 0,
            gpu: s.group("gpu")?.max ?? 0,
            fans: s.fans.map(\.rpm),
            cpuUsage: s.cpuUsage ?? 0,
            gpuUsage: s.gpuUsage ?? 0,
            batteryPct: s.battery?.percent,
            batteryPowerW: s.battery?.powerW
        )
        historyBuffer.append(point)
        if historyBuffer.count > 600 { historyBuffer.removeFirst(historyBuffer.count - 600) }
        // Fenêtres cachées : on ne redessine rien (économie CPU/batterie)
        if uiVisible || snapshot == nil { flushToUI() }
        notifyIfNeeded(s.advice)
    }

    private func notifyIfNeeded(_ a: Advice) {
        defer { lastLevel = a.level }
        guard notificationsEnabled, a.level != lastLevel, ["hot", "critical"].contains(a.level),
              a.action != "none", Date().timeIntervalSince(lastNotified) > 300 else { return }
        lastNotified = Date()
        let c = UNMutableNotificationContent()
        c.title = a.title
        c.body = a.message
        c.sound = .default
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: c, trigger: nil))
    }

    // MARK: Utilisation batterie

    /// Rapport d'utilisation (calculé par le moteur toutes les 30 s ; demandé seulement quand la vue est affichée).
    func fetchUsage() {
        io.async { [weak self] in
            guard let self, let conn = self.conn else { return }
            do {
                let data = try conn.request(["cmd": "usage"])
                let env = try JSONDecoder.snake.decode(UsageEnvelope.self, from: data)
                DispatchQueue.main.async {
                    if let r = env.data {
                        self.usage = r
                        self.usageError = nil
                    } else if !env.ok {
                        self.usageError = "Le helper root installé est une ancienne version : réinstallez-le (pastille en haut à droite › « Réinstaller le helper root… ») pour activer le suivi d'utilisation."
                    } else {
                        self.usageError = nil // pas encore calculé
                    }
                }
            } catch {
                DispatchQueue.main.async { self.usageError = "Rapport illisible : \(error.localizedDescription)" }
            }
        }
    }

    // MARK: Commandes

    func setMode(_ mode: ModeDTO) {
        var m: [String: Any] = ["kind": mode.kind]
        if let p = mode.percent { m["percent"] = p }
        if let p = mode.preset { m["preset"] = p }
        send(["cmd": "set_mode", "mode": m])
    }

    func setCharge(_ cfg: ChargeConfig) { send(["cmd": "set_charge", "config": cfg.json]) }
    func setPmset(_ profile: String, _ key: String, _ value: Int) {
        send(["cmd": "set_pmset", "profile": profile, "key": key, "value": value])
    }
    func powerPreset(_ name: String) { send(["cmd": "power_preset", "name": name]) }

    /// Exécute l'action d'un conseil batterie produit par le moteur.
    func perform(_ action: String) {
        let parts = action.split(separator: ":", maxSplits: 3).map(String.init)
        switch parts.first {
        case "enable_limit":
            var c = snapshot?.charge.config ?? ChargeConfig(enabled: true, limit: 80, heatProtection: true, dischargeToLimit: false)
            c.enabled = true; c.limit = 80
            setCharge(c)
        case "heat_protection":
            if var c = snapshot?.charge.config { c.heatProtection = true; setCharge(c) }
        case "fans_auto": setMode(.auto)
        case "fans_boost": setMode(.curve(.performance))
        case "install_helper": installHelper()
        case "open_battery_settings":
            if let u = URL(string: "x-apple.systempreferences:com.apple.Battery-Settings.extension") { NSWorkspace.shared.open(u) }
        case "pmset" where parts.count == 4:
            if let v = Int(parts[3]) { setPmset(parts[1], parts[2], v) }
        case "quit" where parts.count >= 2:
            quitProcess(pid: Int32(parts[1]) ?? 0, name: parts.count > 2 ? parts[2] : "")
        default: break
        }
    }

    func quitProcess(pid: Int32, name: String) {
        if let app = NSRunningApplication(processIdentifier: pid) {
            if !app.terminate() { commandError = "\(name) a refusé de quitter." }
        } else {
            commandError = "« \(name) » est un processus d'arrière-plan : quittez l'application qui le lance, ou utilisez le Moniteur d'activité."
        }
    }

    private func send(_ req: [String: Any]) {
        commandError = nil
        io.async { [weak self] in
            guard let self, let conn = self.conn else { return }
            do {
                let data = try conn.request(req)
                let ack = try JSONDecoder.snake.decode(AckEnvelope.self, from: data)
                if !ack.ok { DispatchQueue.main.async { self.commandError = ack.error } }
                else { DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.poll() } }
            } catch {
                DispatchQueue.main.async { self.commandError = error.localizedDescription }
            }
        }
    }

    // MARK: Helper root

    func installHelper() {
        guard let bin = engineBinary(), let script = Bundle.main.url(forResource: "install-helper", withExtension: "sh") ?? devScript("install-helper.sh") else {
            commandError = "Script d'installation introuvable."
            return
        }
        runPrivileged("\(sh(script.path)) \(sh(bin.path)) \(getuid())") { [weak self] err in
            self?.commandError = err
            if err == nil {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { self?.reconnect() }
            }
        }
    }

    func uninstallHelper() {
        guard let script = Bundle.main.url(forResource: "uninstall-helper", withExtension: "sh") ?? devScript("uninstall-helper.sh") else { return }
        runPrivileged(sh(script.path)) { [weak self] err in
            self?.commandError = err
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { self?.reconnect() }
        }
    }

    func reconnect() {
        io.async { [weak self] in
            self?.conn = nil
            DispatchQueue.main.async { self?.connect() }
        }
    }

    private func devScript(_ name: String) -> URL? {
        let exe = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
        let u = exe.appendingPathComponent("../../../../scripts/\(name)").standardized
        return FileManager.default.fileExists(atPath: u.path) ? u : nil
    }

    private func sh(_ s: String) -> String { "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'" }

    private func runPrivileged(_ command: String, completion: @escaping @MainActor (String?) -> Void) {
        let escaped = command.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let source = "do shell script \"\(escaped)\" with administrator privileges"
        DispatchQueue.global().async {
            var errInfo: NSDictionary?
            NSAppleScript(source: source)?.executeAndReturnError(&errInfo)
            let msg = errInfo?[NSAppleScript.errorMessage] as? String
            let cancelled = (errInfo?[NSAppleScript.errorNumber] as? Int) == -128
            DispatchQueue.main.async { completion(cancelled ? nil : msg) }
        }
    }
}

extension JSONDecoder {
    static let snake: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()
}
