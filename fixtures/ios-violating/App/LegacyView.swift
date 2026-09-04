import UIKit
import AdSupport

final class LegacyViewController: UIViewController {
    private let webView = UIWebView()
    private let defaults = UserDefaults.standard

    override func viewDidLoad() {
        super.viewDidLoad()
        view.addSubview(webView)
        defaults.set(true, forKey: "seenLegacyScreen")
        let identifier = ASIdentifierManager.shared().advertisingIdentifier
        print(identifier.uuidString)
    }
}
