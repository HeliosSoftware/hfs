enum HFSSidebarSection: String, CaseIterable, Identifiable {
    case server = "Server"
    case fhir = "FHIR"
    case operations = "Operations"

    var id: String { rawValue }

    var destinations: [HFSAdminDestination] {
        HFSAdminDestination.allCases.filter { $0.section == self }
    }
}
