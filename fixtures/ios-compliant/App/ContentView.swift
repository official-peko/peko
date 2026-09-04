import SwiftUI
import WebKit

struct ContentView: View {
    @State private var lastOpened = UserDefaults.standard.string(forKey: "lastOpened")

    var body: some View {
        VStack {
            Text(lastOpened ?? "Welcome")
            WebViewContainer(url: URL(string: "https://peko.dev")!)
        }
    }
}

struct WebViewContainer: UIViewRepresentable {
    let url: URL

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView()
        view.load(URLRequest(url: url))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}
}
