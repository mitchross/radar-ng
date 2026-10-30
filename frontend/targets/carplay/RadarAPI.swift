import Foundation

enum RadarAPI {
    static var serverURL: String { RadarShared.serverURL }

    static func latestFrame() async throws -> RadarFrame {
        var request = URLRequest(url: URL(string: "\(serverURL)/api/manifest.json")!)
        request.timeoutInterval = 10
        request.cachePolicy = .reloadIgnoringLocalCacheData
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
        // The overlay needs the layer's palettes to honour the user's choice.
        guard let frame = try RadarFrame.latest(inManifest: data), frame.palettes?.isEmpty == false else {
            throw URLError(.cannotParseResponse)
        }
        return frame
    }
}
