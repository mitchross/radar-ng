import MapKit

final class RadarTileOverlay: MKTileOverlay {
    let frame: RadarFrame
    /// Identifies CarPlay traffic in server logs; the scene shares the app's process.
    static let userAgent = "RadarNG-CarPlay/\(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?")"
    let palette: String
    /// Read once so every tile of this overlay comes from one server.
    private let server = RadarAPI.serverURL
    var onFailure: (() -> Void)?
    /// Optional diagnostics receive the exact tile data delivered to MapKit.
    var onTileLoaded: ((MKTileOverlayPath, Data) -> Void)?
    /// Every requested tile's outcome, on the main queue; drives the status button.
    var onTileStatus: ((MKTileOverlayPath, RadarTileStatus) -> Void)?

    init(frame: RadarFrame) {
        self.frame = frame
        self.palette = frame.palette
        super.init(urlTemplate: nil)
        canReplaceMapContent = false
        tileSize = CGSize(width: 256, height: 256)
        // The server has z4-7 only; RadarMapViewport keeps the view at or above z4.
        minimumZ = 4
        // Navigation zooms far beyond the radar's native resolution. Serve
        // enlarged parent tiles there, rather than dropping the overlay.
        maximumZ = 19
    }

    override func url(forTilePath path: MKTileOverlayPath) -> URL {
        // Radar NG publishes XYZ tiles; the old IEM feed required inverted Y.
        // loadTile only asks for zooms 4...7; MapKit may still ask for the URL of an overzoomed
        // tile it never fetches, so fall back to the server root rather than crash.
        frame.tileURL(server: server, palette: palette, z: path.z, x: path.x, y: path.y) ?? URL(string: server)!
    }

    override func loadTile(at path: MKTileOverlayPath, result: @escaping (Data?, Error?) -> Void) {
        let level = min(path.z, min(7, max(4, frame.maxZoom ?? 7)))
        let factor = 1 << (path.z - level)
        let source = MKTileOverlayPath(x: path.x / factor, y: path.y / factor,
                                       z: level, contentScaleFactor: path.contentScaleFactor)
        RadarParentTiles.shared.fetch(url(forTilePath: source)) { [weak self] parent in
            self?.deliver(parent, requested: path, factor: factor, result: result)
        }
    }

    private func deliver(_ parent: RadarParentTiles.Entry, requested: MKTileOverlayPath, factor: Int,
                         result: @escaping (Data?, Error?) -> Void) {
        var status = parent.status
        var data: Data?
        if status == .image, let image = parent.image {
            data = factor == 1 ? parent.data : Self.crop(image, x: requested.x, y: requested.y, factor: factor)
            if data == nil { status = .failure }
        }
        let delivered = status, tile = data  // constants: the main-queue closure must not capture vars
        DispatchQueue.main.async { [weak self] in
            self?.onTileStatus?(requested, delivered)
            if let tile { self?.onTileLoaded?(requested, tile) }
            if delivered == .failure { self?.onFailure?() }
        }
        // 204/404 is a dry tile (RadarTileStatus); anything else reports, never silently clear.
        switch delivered {
        case .image: result(tile, nil)
        case .clear: result(nil, nil)
        case .failure: result(nil, URLError(.badServerResponse))
        }
    }

    /// Crops the child's square out of the parent before scaling, so the work is one 256x256 bitmap.
    private static func crop(_ image: CGImage, x: Int, y: Int, factor: Int) -> Data? {
        let square = RadarMapViewport.overzoomCrop(childX: x, childY: y, factor: factor, sourceSize: image.width)
        guard let piece = image.cropping(to: CGRect(x: square.x, y: square.y, width: square.size, height: square.size))
        else { return nil }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        return UIGraphicsImageRenderer(size: CGSize(width: 256, height: 256), format: format).pngData { context in
            context.cgContext.interpolationQuality = .high  // smooth the enlarged radar cells
            UIImage(cgImage: piece).draw(in: CGRect(x: 0, y: 0, width: 256, height: 256))
        }
    }
}

/// Parent tiles shared by every overzoomed child: one download per parent, decoded once.
final class RadarParentTiles: @unchecked Sendable {
    static let shared = RadarParentTiles()

    final class Entry {
        let status: RadarTileStatus
        let data: Data?
        let image: CGImage?
        init(status: RadarTileStatus, data: Data? = nil, image: CGImage? = nil) {
            self.status = status; self.data = data; self.image = image
        }
    }

    private let cache = NSCache<NSString, Entry>()
    private let lock = NSLock()
    private var waiting: [String: [(Entry) -> Void]] = [:]

    private init() { cache.countLimit = 128 }  // URLs carry the frame path, so old frames age out

    func fetch(_ url: URL, completion: @escaping (Entry) -> Void) {
        let key = url.absoluteString
        if let hit = cache.object(forKey: key as NSString) { completion(hit); return }
        lock.lock()
        let first = waiting[key] == nil
        waiting[key, default: []].append(completion)
        lock.unlock()
        guard first else { return }  // this parent is already downloading
        var request = URLRequest(url: url)
        request.timeoutInterval = 8
        request.setValue(RadarTileOverlay.userAgent, forHTTPHeaderField: "User-Agent")
        URLSession.shared.dataTask(with: request) { [weak self] data, response, error in
            let status = error == nil ? RadarTileStatus.of((response as? HTTPURLResponse)?.statusCode) : .failure
            var entry = Entry(status: status == .image ? .failure : status)
            if status == .image, let data, let image = UIImage(data: data)?.cgImage {
                entry = Entry(status: .image, data: data, image: image)
            }
            self?.finish(key, entry)
        }.resume()
    }

    private func finish(_ key: String, _ entry: Entry) {
        if entry.status != .failure { cache.setObject(entry, forKey: key as NSString) }  // failures retry
        lock.lock()
        let callbacks = waiting.removeValue(forKey: key) ?? []
        lock.unlock()
        callbacks.forEach { $0(entry) }
    }
}
