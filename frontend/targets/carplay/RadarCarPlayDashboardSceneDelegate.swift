import CarPlay
import UIKit

@objc(RadarCarPlayDashboardSceneDelegate)
final class RadarCarPlayDashboardSceneDelegate: UIResponder, CPTemplateApplicationDashboardSceneDelegate {
    private var mapController: RadarMapController?
    private var observer: UUID?
    private var shownMessage: String?

    func templateApplicationDashboardScene(_ scene: CPTemplateApplicationDashboardScene,
        didConnect dashboardController: CPDashboardController, to window: UIWindow) {
        let controller = RadarMapController(dashboard: true)
        mapController = controller
        window.rootViewController = controller
        controller.start()
        shownMessage = nil  // a new dashboard has no buttons yet
        observer = RadarDrivingSession.shared.observe { [weak self, weak dashboardController, weak controller] in
          // The subtitle is the only thing that changes; skip the rebuild on every fix and tick.
          let message = RadarDrivingSession.shared.radarMessage
          guard message != self?.shownMessage else { return }
          self?.shownMessage = message
          dashboardController?.shortcutButtons = [
            CPDashboardButton(titleVariants: ["Nearby radar"], subtitleVariants: ["Recenter"],
                image: UIImage(systemName: "location.fill")!) { [weak controller] _ in controller?.recenter() },
            CPDashboardButton(titleVariants: ["Refresh radar"], subtitleVariants: [message],
                image: UIImage(systemName: "arrow.clockwise")!) { _ in RadarDrivingSession.shared.refreshRadar() },
        ]
        }
    }

    func templateApplicationDashboardScene(_ scene: CPTemplateApplicationDashboardScene,
        didDisconnect dashboardController: CPDashboardController, from window: UIWindow) {
        mapController?.stop(); mapController = nil
        if let observer { RadarDrivingSession.shared.removeObserver(observer) }
        observer = nil
        window.rootViewController = nil
    }
}
