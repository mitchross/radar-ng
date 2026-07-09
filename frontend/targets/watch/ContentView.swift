import SwiftUI

struct ContentView: View {
    @EnvironmentObject var store: WatchStore

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 12) {
                    if let alert = store.alerts.first {
                        AlertBadge(alert: alert)
                    }
                    if let f = store.forecast {
                        CurrentCard(current: f.current)
                        NowcastCard(minutely: f.minutely_15)
                        HourlyRow(hourly: f.hourly)
                        DailyList(daily: f.daily)
                    } else if store.isLoading {
                        ProgressView().padding(.top, 30)
                    } else if let err = store.errorMessage {
                        Text(err).font(.footnote).foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 6)
            }
            .navigationTitle("Radar")
            .refreshable { await store.refresh() }
        }
    }
}

struct CurrentCard: View {
    let current: Forecast.Current
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("\(Int(current.temperature_2m.rounded()))°")
                .font(.system(size: 52, weight: .thin, design: .rounded))
            Text(current.weather_code.map { WeatherCodes.label($0) } ?? "—")
                .font(.footnote).foregroundStyle(.secondary)
            HStack(spacing: 10) {
                if let wind = current.wind_speed_10m {
                    Label("\(Int(wind.rounded())) mph", systemImage: "wind")
                }
                Label("\(Int(current.relative_humidity_2m.rounded()))%", systemImage: "humidity")
            }
            .font(.caption2).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(Color.black.opacity(0.25), in: RoundedRectangle(cornerRadius: 14))
    }
}

struct NowcastCard: View {
    let minutely: Forecast.Minutely?
    var body: some View {
        let values = minutely?.precipitation.prefix(60).map { $0 } ?? []
        let peak = values.max() ?? 0
        VStack(alignment: .leading, spacing: 6) {
            Text(peak > 0 ? "RAIN IN NEXT HOUR" : "NO RAIN NEXT HOUR")
                .font(.caption2).bold().foregroundStyle(Color(red: 0.55, green: 0.49, blue: 1.0))
            GeometryReader { geo in
                HStack(alignment: .bottom, spacing: 1) {
                    ForEach(Array(values.enumerated()), id: \.offset) { _, v in
                        let h = min(max(CGFloat(v) * 30, 2), geo.size.height)
                        RoundedRectangle(cornerRadius: 1)
                            .fill(Color(red: 0.55, green: 0.49, blue: 1.0).opacity(0.9))
                            .frame(height: h)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .bottom)
            }
            .frame(height: 28)
        }
        .padding(10)
        .background(Color.black.opacity(0.25), in: RoundedRectangle(cornerRadius: 14))
    }
}

struct HourlyRow: View {
    let hourly: Forecast.Hourly
    var body: some View {
        let items = zip(hourly.time, zip(hourly.temperature_2m, hourly.weather_code)).prefix(12)
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(Array(items.enumerated()), id: \.offset) { _, triple in
                    VStack(spacing: 2) {
                        Text(shortHour(triple.0)).font(.caption2).foregroundStyle(.secondary)
                        Image(systemName: triple.1.1.map { WeatherCodes.sfSymbol($0) } ?? "questionmark")
                            .font(.footnote)
                        Text("\(Int(triple.1.0.rounded()))°").font(.caption2)
                    }
                    .frame(width: 34)
                }
            }
        }
    }

    private func shortHour(_ iso: String) -> String {
        let df = ISO8601DateFormatter()
        df.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let d = df.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) else { return "" }
        let out = DateFormatter()
        out.dateFormat = "ha"
        return out.string(from: d).lowercased()
    }
}

struct DailyList: View {
    let daily: Forecast.Daily
    var body: some View {
        VStack(spacing: 4) {
            ForEach(0..<min(daily.time.count, 5), id: \.self) { i in
                HStack {
                    Text(dayOfWeek(daily.time[i])).font(.caption2).frame(width: 40, alignment: .leading)
                    Image(systemName: daily.weather_code[i].map { WeatherCodes.sfSymbol($0) } ?? "questionmark")
                        .font(.footnote)
                    Spacer()
                    Text("\(Int(daily.temperature_2m_min[i].rounded()))° / \(Int(daily.temperature_2m_max[i].rounded()))°")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
        .padding(10)
        .background(Color.black.opacity(0.25), in: RoundedRectangle(cornerRadius: 14))
    }
    private func dayOfWeek(_ iso: String) -> String {
        let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd"
        guard let d = f.date(from: iso) else { return iso }
        let o = DateFormatter(); o.dateFormat = "E"
        return o.string(from: d)
    }
}

struct AlertBadge: View {
    let alert: Alert
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "exclamationmark.triangle.fill")
            Text(alert.event).font(.caption2).lineLimit(2)
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(severityColor.opacity(0.85), in: RoundedRectangle(cornerRadius: 10))
        .foregroundStyle(.white)
    }
    var severityColor: Color {
        switch alert.severity {
        case "Extreme": return .red
        case "Severe": return .orange
        case "Moderate": return .yellow
        default: return .gray
        }
    }
}

enum WeatherCodes {
    static func label(_ c: Int) -> String {
        switch c {
        case 0: return "Clear"
        case 1,2: return "Mostly Clear"
        case 3: return "Overcast"
        case 45,48: return "Fog"
        case 51,53,55,56,57: return "Drizzle"
        case 61,63,65,66,67,80,81,82: return "Rain"
        case 71,73,75,77,85,86: return "Snow"
        case 95,96,99: return "Thunderstorm"
        default: return "—"
        }
    }
    static func sfSymbol(_ c: Int) -> String {
        switch c {
        case 0: return "sun.max.fill"
        case 1,2: return "cloud.sun.fill"
        case 3: return "cloud.fill"
        case 45,48: return "cloud.fog.fill"
        case 51,53,55,56,57,61,63,65,66,67,80,81,82: return "cloud.rain.fill"
        case 71,73,75,77,85,86: return "cloud.snow.fill"
        case 95,96,99: return "cloud.bolt.rain.fill"
        default: return "cloud.fill"
        }
    }
}
