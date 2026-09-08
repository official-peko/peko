import SwiftUI
import StoreKit

struct PaywallView: View {
    @EnvironmentObject private var session: Session
    @Environment(\.dismiss) private var dismiss
    @State private var products: [Product] = []
    @State private var buying = false

    var body: some View {
        VStack(spacing: 24) {
            Text("Harbor Pro")
                .font(.largeTitle.bold())

            VStack(alignment: .leading, spacing: 8) {
                Label("Sync across every device", systemImage: "checkmark")
                Label("Unlimited attachments", systemImage: "checkmark")
                Label("Search inside images", systemImage: "checkmark")
            }

            ForEach(products, id: \.id) { product in
                Button {
                    Task { await buy(product) }
                } label: {
                    Text("Subscribe for \(product.displayPrice)")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(buying)
            }

            Button("Not now") { dismiss() }
                .font(.footnote)
        }
        .padding(32)
        .task {
            products = (try? await Product.products(for: ["com.example.harbor.pro.monthly"])) ?? []
        }
    }

    private func buy(_ product: Product) async {
        buying = true
        defer { buying = false }
        guard let result = try? await product.purchase() else { return }
        if case .success = result {
            session.isSubscribed = true
            Analytics.shared.track("subscribed", ["price": product.displayPrice])
        }
    }
}
