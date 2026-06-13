import SwiftUI

struct HFSPlaceholderTile: View {
    var tile: HFSPlaceholderTileModel

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    Text(tile.value)
                        .font(.title3.weight(.semibold))
                        .monospacedDigit()
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    Image(systemName: tile.systemImage)
                        .foregroundStyle(.secondary)
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text(tile.title)
                        .font(.headline)
                        .lineLimit(1)
                    Text(tile.caption)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            .frame(minHeight: 104, alignment: .topLeading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
