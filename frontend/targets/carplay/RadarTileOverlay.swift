import MapKit

final class RadarTileOverlay: MKTileOverlay {
    let frame: RadarAPI.Frame
    /// Identifies CarPlay traffic in server logs; the scene shares the app's process.
    static let userAgent = "RadarNG-CarPlay/\(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?")"
    let palette: String
    var onFailure: (() -> Void)?
    /// Optional diagnostics receive the exact tile data delivered to MapKit.
    var onTileLoaded: ((MKTileOverlayPath, Data) -> Void)?

    init(frame: RadarAPI.Frame) {
        self.frame = frame
        self.palette = RadarShared.palette(among: frame.palettes ?? [])
        super.init(urlTemplate: nil)
        canReplaceMapContent = false
        tileSize = CGSize(width: 256, height: 256)
        minimumZ = 4
        // Navigation zooms far beyond the radar's native resolution. Serve
        // enlarged parent tiles there, rather than dropping the overlay.
        maximumZ = 19
    }

    override func url(forTilePath path: MKTileOverlayPath) -> URL {
        // Radar NG publishes XYZ tiles; the old IEM feed required inverted Y.
        URL(string: "\(RadarAPI.serverURL)/tiles/radar/\(palette)/\(frame.path)/\(path.z)/\(path.x)/\(path.y).png")!
    }

    override func loadTile(at path: MKTileOverlayPath, result: @escaping (Data?, Error?) -> Void) {
        let level = min(path.z, min(7, max(4, frame.max_zoom ?? 7)))
        let factor = 1 << (path.z - level)
        let source = MKTileOverlayPath(x: path.x / factor, y: path.y / factor,
                                       z: level, contentScaleFactor: path.contentScaleFactor)
        load(source, requested: path, onlyNotFound: true, result: result)
    }

    private func load(_ path: MKTileOverlayPath, requested: MKTileOverlayPath, onlyNotFound: Bool,
                      result: @escaping (Data?, Error?) -> Void) {
        var request = URLRequest(url: url(forTilePath: path))
        request.timeoutInterval = 8
        request.setValue(Self.userAgent, forHTTPHeaderField: "User-Agent")
        URLSession.shared.dataTask(with: request) { [weak self] data, response, error in
            let notFound = error == nil && (response as? HTTPURLResponse)?.statusCode == 404
            if error == nil, (response as? HTTPURLResponse)?.statusCode == 200,
               let data, let image = UIImage(data: data)?.cgImage {
                if path.z == requested.z {
                    DispatchQueue.main.async { self?.onTileLoaded?(requested, data) }
                    result(data, nil); return
                }
                let factor = 1 << (requested.z - path.z)
                let format = UIGraphicsImageRendererFormat()
                format.scale = 1
                let cropped = UIGraphicsImageRenderer(size: CGSize(width: 256, height: 256), format: format).image { _ in
                    UIImage(cgImage: image).draw(in: CGRect(
                        x: -CGFloat(requested.x % factor) * 256,
                        y: -CGFloat(requested.y % factor) * 256,
                        width: CGFloat(factor) * 256, height: CGFloat(factor) * 256))
                }
                if let png = cropped.pngData() {
                    DispatchQueue.main.async { self?.onTileLoaded?(requested, png) }
                    result(png, nil)
                    return
                }
            }
            // Sparse high-zoom coverage is backed by the same observation's
            // lower-resolution tile. The tiler writes no file for a tile without
            // precipitation, so 404 at every level means clear; any other
            // failure still reports rather than silently turning clear.
            let stillOnlyNotFound = onlyNotFound && notFound
            if path.z > 4, let self {
                let parent = MKTileOverlayPath(x: path.x / 2, y: path.y / 2,
                                               z: path.z - 1, contentScaleFactor: path.contentScaleFactor)
                self.load(parent, requested: requested, onlyNotFound: stillOnlyNotFound, result: result)
            } else if stillOnlyNotFound {
                result(nil, nil)  // clear: nothing to draw
            } else {
                DispatchQueue.main.async { self?.onFailure?() }
                result(nil, error ?? URLError(.badServerResponse))
            }
        }.resume()
    }
}
