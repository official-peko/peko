import XCTest
import UIKit

// A test file. It holds a deliberate violation that no store ever sees,
// because the test bundle does not ship.
final class HelperTests: XCTestCase {
    func testLegacyWebView() {
        let view = UIWebView()
        XCTAssertNotNil(view)
    }
}
