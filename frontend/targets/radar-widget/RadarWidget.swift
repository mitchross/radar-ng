import SwiftUI
import WidgetKit

struct RadarEntry: TimelineEntry {
    let date: Date
    var image: UIImage?
    var observedAt: Date?
    var message: String
    var symbol = "cloud.rain"
}

struct RadarProvider: TimelineProvider {
    func placeholder(in context: Context) -> RadarEntry {
        RadarEntry(date: .now, message: "Nearby radar")
    }

    func getSnapshot(in context: Context, completion: @escaping (RadarEntry) -> Void) {
        if context.isPreview {
            completion(placeholder(in: context))
        } else {
            Task { @MainActor in completion(await RadarSnapshot.load()) }
        }
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<RadarEntry>) -> Void) {
        Task { @MainActor in
            let entry = await RadarSnapshot.load()
            // This is an earliest refresh request, never a promise of live updates.
            // A future entry marks an old frame even if the next fetch is delayed.
            var entries = [entry]
            if let observedAt = entry.observedAt {
                let staleAt = observedAt.addingTimeInterval(15 * 60)
                if staleAt > entry.date {
                    entries.append(RadarEntry(date: staleAt, image: entry.image,
                                              observedAt: observedAt, message: "Older radar"))
                }
            }
            completion(Timeline(entries: entries, policy: .after(Date().addingTimeInterval(600))))
        }
    }
}

struct RadarWidgetView: View {
    let entry: RadarEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Label(entry.message == "Older radar" ? "OLDER RADAR" : "RADAR", systemImage: "dot.radiowaves.left.and.right")
                .font(.caption.weight(.bold))
                .foregroundStyle(.secondary)
            if let image = entry.image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .clipShape(RoundedRectangle(cornerRadius: 9))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityLabel("Precipitation radar near your sampled location")
            } else {
                Image(systemName: entry.symbol)
                    .font(.largeTitle)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .foregroundStyle(.secondary)
            }
            if entry.image == nil {
                Text(entry.message)
                    .font(.caption.weight(.semibold))
                    .lineLimit(2)
                    .minimumScaleFactor(0.85)
            }
            if let observedAt = entry.observedAt {
                Text("Snapshot · \(observedAt, style: .time)")
                .font(.caption2).foregroundStyle(.secondary)
                .lineLimit(1).minimumScaleFactor(0.8)
                .accessibilityLabel("Radar observed \(observedAt.formatted(date: .complete, time: .shortened))")
            }
        }
        .containerBackground(.background, for: .widget)
        .widgetURL(URL(string: "radarng://"))
    }
}

#if !WIDGET_TEST_HOST
@main
#endif
struct RadarWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "RadarSnapshot", provider: RadarProvider()) { entry in
            RadarWidgetView(entry: entry)
        }
        .configurationDisplayName("Nearby Radar")
        .description("A timestamped precipitation snapshot for your location. Updates periodically, including in CarPlay.")
        .supportedFamilies([.systemSmall])
    }
}
