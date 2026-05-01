import MapKit

final class RadarTileOverlay: MKTileOverlay {
    var opacity: CGFloat = 0.7
    private var cacheKey: Int = Int(Date().timeIntervalSince1970)

    init() {
        super.init(urlTemplate: nil)
        self.canReplaceMapContent = false
        self.tileSize = CGSize(width: 256, height: 256)
        self.minimumZ = 1
        self.maximumZ = 12
    }

    func bumpCacheKey() {
        cacheKey = Int(Date().timeIntervalSince1970)
    }

    override func url(forTilePath path: MKTileOverlayPath) -> URL {
        let tmsY = (1 << path.z) - 1 - path.y
        let s = "https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0/nexrad-n0q-0/\(path.z)/\(path.x)/\(tmsY).png?c=\(cacheKey)"
        return URL(string: s)!
    }
}
