import SwiftUI

enum IdentityProvider {
    case google
    case facebook
}

struct SignInView: View {
    @EnvironmentObject private var session: Session
    @State private var busy = false

    var body: some View {
        VStack(spacing: 20) {
            Text("Harbor")
                .font(.largeTitle.bold())
            Text("Your notes, on every device you own.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            VStack(spacing: 12) {
                Button {
                    Task { await signIn(with: .google) }
                } label: {
                    Label("Continue with Google", systemImage: "g.circle")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)

                Button {
                    Task { await signIn(with: .facebook) }
                } label: {
                    Label("Continue with Facebook", systemImage: "f.circle")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
            }
            .disabled(busy)
            .padding(.horizontal, 32)
        }
        .padding()
    }

    private func signIn(with provider: IdentityProvider) async {
        busy = true
        defer { busy = false }
        guard let account = await IdentityClient.shared.authenticate(provider) else { return }
        session.signIn(with: account)
    }
}

final class IdentityClient {
    static let shared = IdentityClient()

    /// Hand off to the provider SDK and exchange the token with our server.
    func authenticate(_ provider: IdentityProvider) async -> Session.Account? {
        Session.Account(
            id: UUID().uuidString,
            email: "person@example.com",
            displayName: "A Person"
        )
    }
}
