import SwiftUI

@main
struct RadarWatchApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var store = WatchStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(store)
                .task {
                    // A new server, palette or city on the iPhone refreshes the Watch.
                    WatchLink.shared.onChange = { Task { await store.refresh() } }
                    WatchLink.shared.activate()
                    await store.refresh()
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active && Date().timeIntervalSince(store.updatedAt ?? .distantPast) > 120 {
                        Task { await store.refresh() }
                    }
                }
        }
    }
}
