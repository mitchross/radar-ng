import SwiftUI

struct ContentView: View {
    var body: some View {
        TabView {
            RadarMapView()
            ForecastPage()
        }
        .tabViewStyle(.verticalPage)
    }
}

struct ForecastPage: View {
    @EnvironmentObject var store: WatchStore

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 12) {
                    if let notice = store.locationNotice {
                        Label(notice, systemImage: "location.slash")
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                    if store.forecast != nil, store.errorMessage != nil {
                        Label("Saved forecast", systemImage: "wifi.slash")
                            .font(.caption2).foregroundStyle(.orange)
                    }
                    if let alert = store.alerts.first {
                        AlertBadge(alert: alert)
                    }
                    if let f = store.forecast {
                        CurrentCard(current: f.current)
                        NowcastCard(minutely: f.minutely_15, currentTime: f.current.time)
                        HourlyRow(hourly: f.hourly, currentTime: f.current.time)
                        DailyList(daily: f.daily)
                    } else if store.isLoading {
                        ProgressView().padding(.top, 30)
                    } else if let err = store.errorMessage {
                        Text(err).font(.footnote).foregroundStyle(.secondary)
                    }
                    Button("Refresh weather", systemImage: "arrow.clockwise") {
                        Task { await store.refresh() }
                    }
                    .disabled(store.isLoading)
                    .accessibilityIdentifier("watch-forecast-refresh")
                }
                .padding(.horizontal, 6)
            }
            .navigationTitle("Forecast")
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
            Text(current.weather_code.map { WeatherCodes.label($0) } ?? "Conditions unavailable")
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
        .background(Color(white: 0.12), in: RoundedRectangle(cornerRadius: 14))
    }
}

struct NowcastCard: View {
    let minutely: Forecast.Minutely?
    let currentTime: String
    var body: some View {
        let values = WatchForecastPresentation.nextHour(minutely, currentTime: currentTime)
        let peak = values.max() ?? 0
        VStack(alignment: .leading, spacing: 6) {
            Text(values.isEmpty ? "RAIN DATA UNAVAILABLE" : peak > 0 ? "RAIN IN NEXT HOUR" : "NO RAIN NEXT HOUR")
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
        .background(Color(white: 0.12), in: RoundedRectangle(cornerRadius: 14))
    }
}

struct HourlyRow: View {
    let hourly: Forecast.Hourly
    let currentTime: String
    var body: some View {
        let items = zip(hourly.time, zip(hourly.temperature_2m, hourly.weather_code))
            .filter { $0.0 >= String(currentTime.prefix(13)) + ":00" }.prefix(12)
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(Array(items.enumerated()), id: \.offset) { _, triple in
                    VStack(spacing: 2) {
                        Text(WatchForecastPresentation.shortHour(triple.0)).font(.caption2).foregroundStyle(.secondary)
                        Image(systemName: triple.1.1.map { WeatherCodes.sfSymbol($0) } ?? "questionmark.circle")
                            .font(.footnote)
                        Text("\(Int(triple.1.0.rounded()))°").font(.caption2)
                    }
                    .frame(width: 34)
                }
            }
        }
    }


}

struct DailyList: View {
    let daily: Forecast.Daily
    var body: some View {
        VStack(spacing: 4) {
            ForEach(0..<min(daily.time.count, daily.weather_code.count, daily.temperature_2m_min.count, daily.temperature_2m_max.count, 5), id: \.self) { i in
                HStack {
                    Text(WatchForecastPresentation.dayOfWeek(daily.time[i])).font(.caption2).frame(width: 40, alignment: .leading)
                    Image(systemName: daily.weather_code[i].map { WeatherCodes.sfSymbol($0) } ?? "questionmark.circle")
                        .font(.footnote)
                    Spacer()
                    Text("\(Int(daily.temperature_2m_min[i].rounded()))° / \(Int(daily.temperature_2m_max[i].rounded()))°")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
        .padding(10)
        .background(Color(white: 0.12), in: RoundedRectangle(cornerRadius: 14))
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
        .foregroundStyle(alert.severity == "Moderate" ? .black : .white)
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
