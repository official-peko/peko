import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var session: Session
    @AppStorage("harbor.sync.wifiOnly") private var wifiOnly = true

    var body: some View {
        Form {
            if let account = session.account {
                Section("Account") {
                    LabeledContent("Name", value: account.displayName)
                    LabeledContent("Email", value: account.email)
                }
            }

            Section("Sync") {
                Toggle("Only sync on Wi-Fi", isOn: $wifiOnly)
            }

            Section {
                Link("Privacy policy", destination: URL(string: "https://harborapp.example/privacy")!)
                Link("Support", destination: URL(string: "https://harborapp.example/support")!)
                Link(
                    "Do Not Sell or Share My Personal Information",
                    destination: URL(string: "https://harborapp.example/do-not-sell")!
                )
            }

            Section {
                Button("Sign out", role: .destructive) {
                    session.signOut()
                }
            }
        }
        .navigationTitle("Settings")
    }
}
