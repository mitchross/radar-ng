import Foundation
import CoreLocation
import Combine

@MainActor
final class WatchStore: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published var forecast: Forecast?
    @Published var alerts: [Alert] = []
    @Published var isLoading = false
    @Published var errorMessage: String?
    @Published var location: CLLocationCoordinate2D?
    @Published var radarFrame: WatchRadarFrame?
    @Published var radarErrorMessage: String?
    @Published var isRadarLoading = false

    private let clManager = CLLocationManager()
    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)

    override init() {
        super.init()
        clManager.delegate = self
        clManager.desiredAccuracy = kCLLocationAccuracyKilometer
    }

    func refresh() async {
        isLoading = true
        isRadarLoading = true
        defer {
            isLoading = false
            isRadarLoading = false
        }
        if location == nil {
            clManager.requestWhenInUseAuthorization()
            clManager.requestLocation()
        }
        let coord = location ?? fallback
        async let forecastResult = Self.capture { try await WatchAPI.fetchForecast(lat: coord.latitude, lon: coord.longitude) }
        async let alertResult = Self.capture { try await WatchAPI.fetchAlerts(lat: coord.latitude, lon: coord.longitude) }
        async let radarResult = Self.capture { try await WatchAPI.fetchLatestRadarFrame() }

        switch await forecastResult {
        case .success(let forecast):
            self.forecast = forecast
            self.errorMessage = nil
        case .failure(let error):
            self.errorMessage = error.localizedDescription
        }
        if case .success(let alerts) = await alertResult {
            self.alerts = alerts
        }
        apply(await radarResult)
    }

    func refreshRadar() async {
        guard !isRadarLoading else { return }
        isRadarLoading = true
        defer { isRadarLoading = false }
        apply(await Self.capture { try await WatchAPI.fetchLatestRadarFrame() })
    }

    private func apply(_ result: Result<WatchRadarFrame, Error>) {
        switch result {
        case .success(let frame):
            radarFrame = frame
            radarErrorMessage = nil
        case .failure(let error):
            radarErrorMessage = error.localizedDescription
        }
    }

    private static func capture<T>(
        _ operation: () async throws -> T
    ) async -> Result<T, Error> {
        do {
            return .success(try await operation())
        } catch {
            return .failure(error)
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let coord = locations.last?.coordinate else { return }
        Task { @MainActor in
            self.location = coord
            await self.refresh()
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor in await self.refresh() }
    }
}
