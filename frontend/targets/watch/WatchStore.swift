import Foundation
import CoreLocation
import Combine

@MainActor
final class WatchStore: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published var forecast: Forecast?
    @Published var alerts: [Alert] = []
    @Published var alertErrorMessage: String?
    @Published var isLoading = false
    @Published var errorMessage: String?
    @Published var location: CLLocationCoordinate2D?
    @Published var radarFrame: WatchRadarFrame?
    @Published var radarErrorMessage: String?
    @Published var isRadarLoading = false
    @Published var radarRefreshID = 0
    @Published var locationNotice: String?
    @Published var updatedAt: Date?

    private var requestedLocation = false
    private var lastLocation: CLLocation?
    private var locationRevision = 0
    private var pendingLocationRefresh = false
    private let clManager = CLLocationManager()
    private let fallback = CLLocationCoordinate2D(latitude: 42.9634, longitude: -85.6681)

    override init() {
        super.init()
        clManager.delegate = self
        clManager.desiredAccuracy = kCLLocationAccuracyKilometer
    }

    func refresh() async {
        guard !isLoading else { return }
        isLoading = true
        isRadarLoading = true
        defer {
            isLoading = false
            isRadarLoading = false
            if pendingLocationRefresh {
                pendingLocationRefresh = false
                Task { await self.refresh() }
            }
        }
        requestLocationIfNeeded()
        let revision = locationRevision
        let coord = location ?? fallback
        async let forecastResult = Self.capture { try await WatchAPI.fetchForecast(lat: coord.latitude, lon: coord.longitude) }
        async let alertResult = Self.capture { try await WatchAPI.fetchAlerts(lat: coord.latitude, lon: coord.longitude) }
        async let radarResult = Self.capture { try await WatchAPI.fetchLatestRadarFrame() }

        let results = await (forecastResult, alertResult, radarResult)
        apply(results.2)
        guard revision == locationRevision else { return }
        switch results.0 {
        case .success(let forecast):
            self.forecast = forecast
            self.errorMessage = nil
            self.updatedAt = Date()
        case .failure(let error):
            self.errorMessage = error.localizedDescription
        }
        switch results.1 {
        case .success(let alerts):
            self.alerts = alerts.filter { $0.isActive(at: Date()) }
            alertErrorMessage = nil
        case .failure:
            self.alerts = []
            alertErrorMessage = "Alerts unavailable"
        }
    }

    private func requestLocationIfNeeded() {
        if !requestedLocation || lastLocation.map({ Date().timeIntervalSince($0.timestamp) > 120 }) != false {
            requestedLocation = true
            switch clManager.authorizationStatus {
            case .authorizedAlways, .authorizedWhenInUse:
                clManager.requestLocation()
            case .notDetermined:
                clManager.requestWhenInUseAuthorization()
            default:
                locationNotice = location == nil ? "Showing Grand Rapids" : "Using last known location"
            }
        }
    }

    func refreshRadar() async {
        requestLocationIfNeeded()
        guard !isRadarLoading else { return }
        isRadarLoading = true
        defer { isRadarLoading = false }
        apply(await Self.capture { try await WatchAPI.fetchLatestRadarFrame() })
    }

    private func apply(_ result: Result<WatchRadarFrame, Error>) {
        radarRefreshID += 1
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

    nonisolated static func accepts(_ fix: CLLocation, now: Date = .now) -> Bool {
        CLLocationCoordinate2DIsValid(fix.coordinate) && fix.horizontalAccuracy >= 0 &&
        fix.horizontalAccuracy <= 5000 && abs(fix.timestamp.timeIntervalSince(now)) <= 120
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let fix = locations.last(where: { Self.accepts($0) }) else { return }
        let coord = fix.coordinate
        Task { @MainActor in
            guard self.lastLocation == nil || fix.timestamp >= self.lastLocation!.timestamp else { return }
            let previous = self.location
            self.lastLocation = fix
            self.location = coord
            self.locationNotice = nil
            if previous == nil || CLLocation(latitude: previous!.latitude, longitude: previous!.longitude)
                .distance(from: CLLocation(latitude: coord.latitude, longitude: coord.longitude)) > 2000 {
                self.locationRevision += 1
                self.forecast = nil
                self.alerts = []
                self.updatedAt = nil
                if self.isLoading {
                    self.pendingLocationRefresh = true
                } else {
                    await self.refresh()
                }
            }
        }
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let status = manager.authorizationStatus
        Task { @MainActor in
            guard self.requestedLocation else { return }
            if status == .authorizedAlways || status == .authorizedWhenInUse {
                self.clManager.requestLocation()
            } else if status == .denied || status == .restricted {
                self.locationNotice = self.location == nil ? "Showing Grand Rapids" : "Using last known location"
            }
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor in
            // A denied/failed location must never recursively trigger more requests.
            self.locationNotice = self.location == nil ? "Showing Grand Rapids" : "Using last known location"
        }
    }
}
