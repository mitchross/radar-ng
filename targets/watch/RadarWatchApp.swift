import SwiftUI

@main
struct RadarWatchApp: App {
    @StateObject private var store = WatchStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(store)
                .task { await store.refresh() }
        }
    }
}
