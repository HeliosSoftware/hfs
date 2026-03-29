import { useEffect, useState } from 'react'
import { useAuth } from '../auth/AuthProvider'
import { searchPatients, type FhirBundle, type FhirResource } from '../fhir/client'

export function DashboardPage() {
  const { user, logout } = useAuth()
  const [patients, setPatients] = useState<FhirResource[]>([])
  const [total, setTotal] = useState<number | undefined>()
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [query, setQuery] = useState('')

  useEffect(() => {
    loadPatients()
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  async function loadPatients(name?: string) {
    setLoading(true)
    setError(null)
    try {
      const params = name ? { name } : undefined
      const bundle: FhirBundle = await searchPatients(params)
      setTotal(bundle.total)
      setPatients(
        (bundle.entry ?? [])
          .map((e) => e.resource)
          .filter((r): r is FhirResource => r !== undefined)
      )
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }

  function handleSearch(e: React.FormEvent) {
    e.preventDefault()
    loadPatients(query.trim() || undefined)
  }

  const displayName =
    user?.profile?.name ??
    user?.profile?.preferred_username ??
    user?.profile?.sub ??
    'User'

  return (
    <div style={{ maxWidth: '60rem', margin: '0 auto', padding: '2rem' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '2rem' }}>
        <h1 style={{ margin: 0 }}>HFS FHIR Server</h1>
        <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
          <span style={{ fontSize: '0.875rem', color: '#555' }}>Signed in as {displayName}</span>
          <button onClick={logout} style={{ fontSize: '0.875rem', cursor: 'pointer' }}>
            Sign out
          </button>
        </div>
      </header>

      <section>
        <h2>Patients {total !== undefined && <span style={{ fontSize: '1rem', color: '#666' }}>({total} total)</span>}</h2>

        <form onSubmit={handleSearch} style={{ display: 'flex', gap: '0.5rem', marginBottom: '1.5rem' }}>
          <input
            type="search"
            placeholder="Search by name..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{ flex: 1, padding: '0.5rem', fontSize: '1rem' }}
          />
          <button type="submit" style={{ padding: '0.5rem 1rem', cursor: 'pointer' }}>
            Search
          </button>
        </form>

        {error && (
          <p style={{ color: '#c00', marginBottom: '1rem' }}>Error: {error}</p>
        )}

        {loading ? (
          <p>Loading patients...</p>
        ) : patients.length === 0 ? (
          <p style={{ color: '#666' }}>No patients found.</p>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #ddd' }}>
                <th style={{ textAlign: 'left', padding: '0.5rem' }}>ID</th>
                <th style={{ textAlign: 'left', padding: '0.5rem' }}>Name</th>
                <th style={{ textAlign: 'left', padding: '0.5rem' }}>Gender</th>
                <th style={{ textAlign: 'left', padding: '0.5rem' }}>Birth Date</th>
              </tr>
            </thead>
            <tbody>
              {patients.map((p) => (
                <PatientRow key={p.id} patient={p} />
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  )
}

function PatientRow({ patient }: { patient: FhirResource }) {
  const name = extractPatientName(patient)
  return (
    <tr style={{ borderBottom: '1px solid #eee' }}>
      <td style={{ padding: '0.5rem', fontFamily: 'monospace', fontSize: '0.875rem' }}>{patient.id ?? '—'}</td>
      <td style={{ padding: '0.5rem' }}>{name}</td>
      <td style={{ padding: '0.5rem' }}>{String(patient.gender ?? '—')}</td>
      <td style={{ padding: '0.5rem' }}>{String(patient.birthDate ?? '—')}</td>
    </tr>
  )
}

function extractPatientName(patient: FhirResource): string {
  const names = patient.name as Array<{ family?: string; given?: string[] }> | undefined
  if (!names || names.length === 0) return '—'
  const first = names[0]
  const parts = [...(first.given ?? []), first.family].filter(Boolean)
  return parts.join(' ') || '—'
}
