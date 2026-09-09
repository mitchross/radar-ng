import SwiftUI

@main
struct RadarWatchApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var store = WatchStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(store)
                .task { await store.refresh() }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active && Date().timeIntervalSince(store.updatedAt ?? .distantPast) > 120 {
                        Task { await store.refresh() }
                    }
                }
        }
    }
}
