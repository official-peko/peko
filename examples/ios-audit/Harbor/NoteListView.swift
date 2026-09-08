import SwiftUI

struct NoteListView: View {
    @EnvironmentObject private var session: Session
    @StateObject private var store = NoteStore()
    @State private var showingPaywall = false

    var body: some View {
        NavigationStack {
            List(store.notes) { note in
                NavigationLink(note.title) {
                    NoteView(note: note)
                }
            }
            .navigationTitle("Notes")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    NavigationLink {
                        SettingsView()
                    } label: {
                        Image(systemName: "gear")
                    }
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        if session.isSubscribed {
                            store.add()
                        } else if store.notes.count >= 10 {
                            showingPaywall = true
                        } else {
                            store.add()
                        }
                    } label: {
                        Image(systemName: "square.and.pencil")
                    }
                }
            }
            .sheet(isPresented: $showingPaywall) {
                PaywallView()
            }
        }
    }
}

struct NoteView: View {
    let note: Note

    var body: some View {
        ScrollView {
            Text(note.body)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
        }
        .navigationTitle(note.title)
    }
}

struct Note: Identifiable {
    let id = UUID()
    var title: String
    var body: String
}

final class NoteStore: ObservableObject {
    @Published var notes: [Note] = []

    func add() {
        notes.append(Note(title: "New note", body: ""))
    }
}
