import { getUserManager } from '../auth'

const HFS_BASE = import.meta.env.VITE_HFS_BASE_URL ?? '/api'

/** Generic FHIR resource shape. */
export interface FhirResource {
  resourceType: string
  id?: string
  [key: string]: unknown
}

/** FHIR Bundle returned by search operations. */
export interface FhirBundle<T extends FhirResource = FhirResource> {
  resourceType: 'Bundle'
  type: string
  total?: number
  entry?: Array<{ resource?: T; fullUrl?: string }>
}

/** FHIR OperationOutcome for error responses. */
export interface OperationOutcome {
  resourceType: 'OperationOutcome'
  issue: Array<{
    severity: string
    code: string
    diagnostics?: string
  }>
}

/** Error thrown when HFS returns a non-2xx response. */
export class FhirError extends Error {
  readonly status: number
  readonly outcome: OperationOutcome | null

  constructor(status: number, outcome: OperationOutcome | null, message: string) {
    super(message)
    this.name = 'FhirError'
    this.status = status
    this.outcome = outcome
  }
}

/**
 * Make an authenticated request to the HFS FHIR API.
 * Attaches the current access token as a Bearer token.
 */
async function fhirFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const manager = getUserManager()
  const user = await manager.getUser()
  if (!user || user.expired) {
    throw new Error('Not authenticated. Please log in.')
  }

  const url = `${HFS_BASE}${path}`
  const response = await fetch(url, {
    ...init,
    headers: {
      Accept: 'application/fhir+json',
      'Content-Type': 'application/fhir+json',
      Authorization: `Bearer ${user.access_token}`,
      ...init?.headers,
    },
  })

  if (!response.ok) {
    let outcome: OperationOutcome | null = null
    try {
      outcome = await response.json()
    } catch {
      // ignore parse errors
    }
    const diagnostics = outcome?.issue?.[0]?.diagnostics ?? response.statusText
    throw new FhirError(response.status, outcome, `FHIR API error ${response.status}: ${diagnostics}`)
  }

  return response.json() as Promise<T>
}

/** Search for patients. Supports optional FHIR search parameters. */
export async function searchPatients(
  params?: Record<string, string>
): Promise<FhirBundle<FhirResource>> {
  const qs = params ? new URLSearchParams(params).toString() : ''
  return fhirFetch(`/Patient${qs ? '?' + qs : ''}`)
}

/** Read a single Patient by ID. */
export async function readPatient(id: string): Promise<FhirResource> {
  return fhirFetch(`/Patient/${id}`)
}

/** Read the FHIR capability statement. */
export async function readMetadata(): Promise<FhirResource> {
  const manager = getUserManager()
  const user = await manager.getUser()
  const headers: Record<string, string> = { Accept: 'application/fhir+json' }
  if (user && !user.expired) {
    headers['Authorization'] = `Bearer ${user.access_token}`
  }
  const response = await fetch(`${HFS_BASE}/metadata`, { headers })
  return response.json()
}
