import CarPlay
import UIKit

@objc(RadarCarPlaySceneDelegate)
final class RadarCarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate {
    private var interfaceController: CPInterfaceController?
    private var mapController: RadarMapController?

    func templateApplicationScene(_ scene: CPTemplateApplicationScene,
                                  didConnect interfaceController: CPInterfaceController,
                                  to window: CPWindow) {
        self.interfaceController = interfaceController
        let controller = RadarMapController()
        self.mapController = controller
        window.rootViewController = controller
        interfaceController.setRootTemplate(controller.makeTemplate(), animated: true, completion: nil)
        controller.start()
    }

    func templateApplicationScene(_ scene: CPTemplateApplicationScene,
                                  didDisconnect interfaceController: CPInterfaceController,
                                  from window: CPWindow) {
        mapController?.stop()
        mapController = nil
        self.interfaceController = nil
    }
}
