import SwiftUI

@main
struct HarborApp: App {
    @StateObject private var session = Session()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(session)
                .task {
                    Analytics.shared.start()
                }
        }
    }
}

struct RootView: View {
    @EnvironmentObject private var session: Session

    var body: some View {
        if session.account == nil {
            SignInView()
        } else {
            NoteListView()
        }
    }
}

final class Session: ObservableObject {
    @Published var account: Account?
    @Published var isSubscribed = false

    struct Account {
        let id: String
        let email: String
        let displayName: String
    }

    func signIn(with account: Account) {
        self.account = account
        UserDefaults.standard.set(account.id, forKey: "harbor.account.id")
        Analytics.shared.identify(account)
    }

    func signOut() {
        account = nil
        UserDefaults.standard.removeObject(forKey: "harbor.account.id")
    }
}
