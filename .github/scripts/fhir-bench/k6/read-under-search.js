// The `read-under-search` suite of the MongoDB protection ABAB loop
// (.github/scripts/fhir-bench/mongo-protection-abab.sh, issue #2293). HFS's own
// suite, not upstream's: the "Run benchmark suites" step uses a file in this
// directory in place of upstream's k6/<suite>.js.
//
// Why it exists: the MongoDB broad-search gate (HFS_MONGODB_BROAD_SEARCH_CONCURRENCY)
// and the per-search time limit (HFS_MONGODB_SEARCH_TIMEOUT_MS) are meant to keep
// runaway searches from starving everything else. Upstream crud.js does not
// report its reads apart from its writes, so it cannot say whether fast reads
// by id still get served while broad searches are queued or cut off. This suite
// runs both at once and reports them apart.
//
// Load shape: two concurrent constant-vus scenarios, both named after the
// `scenario` tag value k6 puts on every request of that scenario:
//   broad  30 VUs  searches that is_potentially_broad (search_admission.rs)
//                  classifies as broad: a token/string/quantity predicate with
//                  no narrowing value, `_total=accurate`, or a `_sort` on a
//                  field other than _id/_lastUpdated.
//   read   20 VUs  GET Patient/{id} and GET Observation/{id}, ids taken once in
//                  setup() from the existing corpus.
// 30 broad VUs are above a gate of 8 and above the 10-connection pool, so the
// gate (when set) queues and the pool saturates (pool 32 is still below
// 30 + 20, so both pools see contention). k6 --vus / --duration (the
// workflow's vus / duration inputs) replace these scenarios, so a run that
// sets either is not comparable.
//
// Output: requests are tagged scenario:read / scenario:broad. The thresholds
// below always pass; they exist only so k6 --summary-export carries the
// per-scenario http_req_duration, http_reqs and http_req_failed submetrics
// (the same trick as insert.js). A non-2xx answer counts as a failure in
// http_req_failed{scenario:...}; the 408/503/504 split comes from the /metrics
// delta the ABAB script takes around the suite, not from here.
//
// It creates nothing, so it leaves the corpus as it found it.

import http from 'k6/http'
import { check, fail } from 'k6'

export const options = {
  discardResponseBodies: false,
  scenarios: {
    broad: {
      executor: 'constant-vus',
      exec: 'broad',
      vus: 30,
      duration: '2m',
      gracefulStop: '30s',
    },
    read: {
      executor: 'constant-vus',
      exec: 'read',
      vus: 20,
      duration: '2m',
      gracefulStop: '30s',
    },
  },
  thresholds: {
    'http_req_duration{scenario:read}': ['max>=0'],
    'http_req_duration{scenario:broad}': ['max>=0'],
    'http_reqs{scenario:read}': ['count>=0'],
    'http_reqs{scenario:broad}': ['count>=0'],
    'http_req_failed{scenario:read}': ['rate>=0'],
    'http_req_failed{scenario:broad}': ['rate>=0'],
  },
}

const BASE_URL = __ENV.BASE_URL
const HEADERS = {
  'Accept-Encoding': 'gzip',
  'Accept': 'application/fhir+json',
  'Cache-Control': 'no-cache',
}
const ID_POOL = 200

// Broad shapes: no narrowing predicate (see is_potentially_broad), so each one
// takes a gate permit when the gate is on.
const BROAD_SEARCHES = [
  'Patient?gender=female&_count=20',
  'Patient?birthdate=gt1970-01-01&_count=20',
  'Patient?_total=accurate&_count=20',
  'Patient?_sort=birthdate&_count=20',
  'Observation?code=http://loinc.org|8867-4&_count=20',
  'Observation?status=final&_count=20',
  'Observation?_total=accurate&_count=20',
  'Observation?_sort=date&_count=20',
]

function ids(type) {
  const r = http.get(`${BASE_URL}/${type}?_count=${ID_POOL}&_elements=id`, { headers: HEADERS })
  if (r.status !== 200) fail(`setup: ${type} search answered ${r.status}`)
  const out = (r.json('entry') || []).map((e) => e.resource.id)
  if (out.length === 0) fail(`setup: no ${type} in the corpus; run prewarm/import first`)
  return out
}

export function setup() {
  return { Patient: ids('Patient'), Observation: ids('Observation') }
}

export function broad() {
  const q = BROAD_SEARCHES[(__VU + __ITER) % BROAD_SEARCHES.length]
  const r = http.get(`${BASE_URL}/${q}`, { headers: HEADERS, tags: { scenario: 'broad' } })
  check(r, { 'broad search 200': (x) => x.status === 200 })
}

export function read(data) {
  const type = (__ITER % 2 === 0) ? 'Patient' : 'Observation'
  const pool = data[type]
  const id = pool[(__VU * 7 + __ITER) % pool.length]
  const r = http.get(`${BASE_URL}/${type}/${id}`, { headers: HEADERS, tags: { scenario: 'read' } })
  check(r, { 'read by id 200': (x) => x.status === 200 })
}
