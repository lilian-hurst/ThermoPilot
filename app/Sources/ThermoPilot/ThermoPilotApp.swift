import SwiftUI

@main
struct ThermoPilotApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var client = EngineClient()

    var body: some Scene {
        Window("ThermoPilot", id: "main") {
            TabView {
                DashboardView().tabItem { Label(tr("Thermique", "Thermal"), systemImage: "thermometer.medium") }
                BatteryView().tabItem { Label(tr("Batterie", "Battery"), systemImage: "battery.75percent") }
            }
                .id(client.languageChoice) // redessine tout au changement de langue
                .environment(client)
                .frame(minWidth: 900, minHeight: 700)
                .onAppear {
                    client.start()
                    Dock.show()
                }
                .onDisappear { Dock.hide() }
        }
        .defaultSize(width: 1080, height: 860)
        // L'app vit dans la barre des menus : pas de fenêtre au lancement
        .defaultLaunchBehavior(.suppressed)

        MenuBarExtra {
            MenuBarView().id(client.languageChoice).environment(client).onAppear { client.start() }
        } label: {
            MenuBarLabel(client: client)
        }
        .menuBarExtraStyle(.window)
    }
}

extension Notification.Name {
    static let openDashboard = Notification.Name("ThermoPilot.openDashboard")
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Relancer l'app (Spotlight, Finder, Launchpad) alors qu'elle tourne → ouvre le tableau de bord.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        NotificationCenter.default.post(name: .openDashboard, object: nil)
        return false
    }
}

struct MenuBarLabel: View {
    let client: EngineClient
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        HStack(spacing: 2) {
            Image(systemName: client.menuHotspot == nil ? "fan" : Thermal.levelIcon(client.menuLevel))
            Text(client.menuHotspot.map { "\($0)°" } ?? "--")
        }
        .task { client.start() }
        .onReceive(NotificationCenter.default.publisher(for: .openDashboard)) { _ in
            Dock.show()
            openWindow(id: "main")
        }
    }
}

/// Icône du Dock affichée uniquement quand le tableau de bord est ouvert.
@MainActor
enum Dock {
    static func show() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    static func hide() {
        // Laisse la fenêtre se fermer avant de retirer l'icône (sinon macOS peut réactiver une autre fenêtre)
        DispatchQueue.main.async {
            let dashboardOpen = NSApp.windows.contains { $0.isVisible && $0.identifier?.rawValue.contains("main") == true }
            if !dashboardOpen { NSApp.setActivationPolicy(.accessory) }
        }
    }
}
