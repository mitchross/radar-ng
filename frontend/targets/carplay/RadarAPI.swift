import Foundation

enum RadarAPI {
    static var serverURL: String { RadarShared.serverURL }
    // Built once: observedAt runs inside filter/max over every manifest frame.
    fileprivate static let fractionalISO: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
    fileprivate static let plainISO = ISO8601DateFormatter()
    struct Manifest: Decodable { let layers: [String: Layer] }
    struct Layer: Decodable { let frames: [Frame]?; let palettes: [String]? }
    struct Frame: Decodable, Equatable {
        let timestamp: String
        let path: String
        let palettes: [String]?
        let max_zoom: Int?
        var observedAt: Date? {
            RadarAPI.fractionalISO.date(from: timestamp) ?? RadarAPI.plainISO.date(from: timestamp)
        }
        var isValid: Bool {
            guard let date = observedAt, date.timeIntervalSinceNow <= 60 else { return false }
            return !path.isEmpty && path.split(separator: "/").allSatisfy {
                $0 != "." && $0 != ".." && $0.allSatisfy { $0.isNumber || $0.isLetter || "_.:+-".contains($0) }
            }
        }
    }

    static func latestFrame() async throws -> Frame {
        var request = URLRequest(url: URL(string: "\(serverURL)/api/manifest.json")!)
        request.timeoutInterval = 10
        request.cachePolicy = .reloadIgnoringLocalCacheData
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
        let manifest = try JSONDecoder().decode(Manifest.self, from: data)
        guard let layer = manifest.layers["radar"],
              let frame = layer.frames?.filter(\.isValid).max(by: { $0.observedAt! < $1.observedAt! }),
              let palettes = frame.palettes ?? layer.palettes, !palettes.isEmpty else {
            throw URLError(.cannotParseResponse)
        }
        // Carry the layer's palettes so the overlay can honour the user's choice.
        return Frame(timestamp: frame.timestamp, path: frame.path, palettes: palettes, max_zoom: frame.max_zoom)
    }
}
