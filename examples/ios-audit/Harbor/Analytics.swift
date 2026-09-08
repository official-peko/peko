import Foundation

/// Events go to our own metrics service and to the attribution partner.
final class Analytics {
    static let shared = Analytics()

    private let endpoint = URL(string: "https://metrics.harborapp.example/v1/events")!
    private var installId = ""

    func start() {
        installId = Self.installId()
        track("launch", [:])
    }

    func identify(_ account: Session.Account) {
        send([
            "type": "identify",
            "user_id": account.id,
            "email": account.email,
            "name": account.displayName,
            "install_id": installId,
        ])
    }

    func track(_ name: String, _ properties: [String: String]) {
        var body = properties
        body["type"] = "track"
        body["event"] = name
        body["install_id"] = installId
        send(body)
    }

    private static func installId() -> String {
        let key = "harbor.install.id"
        if let existing = UserDefaults.standard.string(forKey: key) {
            return existing
        }
        let fresh = UUID().uuidString
        UserDefaults.standard.set(fresh, forKey: key)
        return fresh
    }

    private func send(_ body: [String: String]) {
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        URLSession.shared.dataTask(with: request).resume()
    }
}
