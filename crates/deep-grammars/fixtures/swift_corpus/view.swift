import SwiftUI

struct ContentView: View {
    @State private var query: String = ""
    @State private var items: [FeedItem] = []
    @StateObject private var model = DownloadsViewModel()

    var body: some View {
        NavigationStack {
            List {
                ForEach(items.indices, id: \.self) { index in
                    row(items[index])
                }
            }
            .navigationTitle("Feed")
            .searchable(text: $query, prompt: "Search")
            .refreshable {
                await model.reload([])
            }
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Reload") {
                        Task { await model.reload([]) }
                    }
                }
            }
        }
        .onAppear {
            items = [.separator("Today")]
        }
    }

    @ViewBuilder
    private func row(_ item: FeedItem) -> some View {
        switch item {
        case .post(let post):
            VStack(alignment: .leading) {
                Text(post.title).font(.headline)
                Text(post.body).font(.subheadline).lineLimit(2)
            }
        case .advertisement(let url, _):
            Link("Sponsored", destination: url)
        case .separator(let title):
            Text(title).foregroundStyle(.secondary)
        }
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
