import SwiftUI
import StoreKit

/// A paywall with no restore control.
///
/// Guideline 3.1.1 asks for a way to restore a subscription on a new device.
/// This view sells one and never offers it back.
struct PaywallView: View {
    @State private var products: [Product] = []

    var body: some View {
        VStack(spacing: 16) {
            Text("Northwind Pro")
                .font(.title)
            ForEach(products, id: \.id) { product in
                Button(product.displayName) {
                    Task { try? await product.purchase() }
                }
            }
        }
        .task {
            products = (try? await Product.products(for: ["com.example.northwind.pro"])) ?? []
        }
    }
}

/// Everything is behind the login, including the part that needs no account.
struct RootView: View {
    @State private var signedIn = false

    var body: some View {
        if signedIn {
            PaywallView()
        } else {
            SignInView(onDone: { signedIn = true })
        }
    }
}

/// Sign in with one social network and nothing else.
struct SignInView: View {
    let onDone: () -> Void

    var body: some View {
        VStack {
            Button("Continue with Facebook", action: onDone)
        }
    }
}
