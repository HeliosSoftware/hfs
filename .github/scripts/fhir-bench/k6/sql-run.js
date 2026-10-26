// The `sql-run` suite of .github/workflows/fhir-benchmark.yml: SQL-on-FHIR
// ViewDefinition runs over the stored corpus (GET /$sql-run). HFS's own suite,
// not upstream's: the "Run benchmark suites" step uses a file in this
// directory in place of upstream's k6/<suite>.js.
//
// Why it exists: no other suite exercises the SQL-on-FHIR runner, which reads
// the stored resources through each backend's own runner (native SQL on
// sqlite and postgres, an aggregation pipeline on mongodb, the primary store's
// runner on the *-elasticsearch legs).
//
// Views: on the resource types with the most entries in the corpus import
// loads (Observation about 42%, Claim and ExplanationOfBenefit about 10% each;
// Patient is only about 1,000, so it would measure a tiny table). A
// ViewDefinition has one resource type, so Claim and ExplanationOfBenefit are
// two views.
//   observation_page    flat Observation, _limit=100
//   observation_large   flat Observation, _limit=10000
//   observation_filter  Observation with a low-cardinality `where` on the
//                       code (four codes, one view each, rotated): the
//                       database has to read rows until the limit is filled
//   claim               flat Claim, _limit=1000
//   eob                 flat ExplanationOfBenefit, _limit=1000
// A request with a flat view and a LIMIT returns after the first rows, so the
// first two views measure "first page", not a scan, and every VU reads the
// same rows (warm cache). The `where` view is the honest scan. Read the
// per-view rows, not one blended figure.
//
// Load shape: constant-vus, 10 VUs for 1 minute (gracefulStop 30s). Every
// request is a heavier read than a search, so 10 is well below search's 30.
// k6 --vus / --duration replace this scenario, as they do the other suites'.
//
// Output: the k6 http_reqs rate is requests/s; `sql_run_rows` counts the
// NDJSON lines returned, so rows/s is its rate. The response body is read
// (responseType text) only to count rows; the server streams NDJSON either
// way. The check 'sql-run rows > 0' fails a 200 with an empty body, which
// would otherwise look very fast. setup() creates the ViewDefinitions and
// teardown() deletes them, so they do not appear in the result-size counts.
//
// Never pass `patient` or `group`: on the *-elasticsearch legs the primary's
// search index is not written, so a compartment filter would return no rows.
// It is read-only, so it runs before insert (after search).

import http from 'k6/http'
import { check } from 'k6'
import { Counter } from 'k6/metrics'

export const options = {
  scenarios: {
    sql_run: {
      executor: 'constant-vus',
      vus: 10,
      duration: '1m',
      gracefulStop: '30s',
    },
  },
  thresholds: {
    'http_req_duration{view:observation_page}': ['max>=0'],
    'http_req_duration{view:observation_large}': ['max>=0'],
    'http_req_duration{view:observation_filter}': ['max>=0'],
    'http_req_duration{view:claim}': ['max>=0'],
    'http_req_duration{view:eob}': ['max>=0'],
  },
}

const BASE_URL = __ENV.BASE_URL
const JSON_HEADERS = { 'Accept': 'application/json', 'Content-Type': 'application/json' }
const rows = new Counter('sql_run_rows')

const FILTER_CODES = ['8302-2', '29463-7', '8867-4', '8480-6']

function view(name, resource, columns, where) {
  const vd = {
    resourceType: 'ViewDefinition',
    name,
    status: 'active',
    resource,
    select: [{ column: columns.map(([n, path]) => ({ name: n, path })) }],
  }
  if (where) vd.where = [{ path: where }]
  return vd
}

const OBSERVATION_COLUMNS = [
  ['id', 'id'],
  ['patient', 'subject.reference'],
  ['code', 'code.coding.first().code'],
  ['value', 'value.ofType(Quantity).value'],
  ['unit', 'value.ofType(Quantity).unit'],
  ['effective', 'effective.ofType(dateTime)'],
]

const CLAIM_COLUMNS = [
  ['id', 'id'],
  ['patient', 'patient.reference'],
  ['status', 'status'],
  ['created', 'created'],
  ['total', 'total.value'],
]

const EOB_COLUMNS = [
  ['id', 'id'],
  ['patient', 'patient.reference'],
  ['status', 'status'],
  ['created', 'created'],
  ['total', 'total.first().amount.value'],
]

function create(vd) {
  const r = http.post(`${BASE_URL}/ViewDefinition`, JSON.stringify(vd), { headers: JSON_HEADERS })
  if (r.status !== 201) throw new Error(`POST ViewDefinition ${vd.name}: ${r.status} ${r.body}`)
  return r.json('id')
}

export function setup() {
  const ids = {
    observation: create(view('bench_observation', 'Observation', OBSERVATION_COLUMNS)),
    claim: create(view('bench_claim', 'Claim', CLAIM_COLUMNS)),
    eob: create(view('bench_eob', 'ExplanationOfBenefit', EOB_COLUMNS)),
    filter: FILTER_CODES.map((code) =>
      create(view(`bench_observation_${code}`, 'Observation', OBSERVATION_COLUMNS,
        `code.coding.first().code = '${code}'`))),
  }
  return ids
}

export function teardown(ids) {
  const all = [ids.observation, ids.claim, ids.eob].concat(ids.filter)
  for (const id of all) http.del(`${BASE_URL}/ViewDefinition/${id}`)
}

function run(id, limit, tag) {
  const url = `${BASE_URL}/$sql-run?subjectReference=ViewDefinition/${id}&_format=ndjson&_limit=${limit}`
  const r = http.get(url, { headers: { 'Accept': 'application/x-ndjson', 'Accept-Encoding': 'gzip' },
    responseType: 'text', tags: { view: tag } })
  const n = r.status === 200 && r.body ? r.body.split('\n').filter((l) => l.length > 0).length : 0
  rows.add(n)
  check(r, {
    'sql-run 200': (x) => x.status === 200,
    'sql-run rows > 0': () => n > 0,
  })
}

export default function (ids) {
  switch ((__VU + __ITER) % 5) {
    case 0: run(ids.observation, 100, 'observation_page'); break
    case 1: run(ids.observation, 10000, 'observation_large'); break
    case 2: run(ids.filter[__ITER % FILTER_CODES.length], 1000, 'observation_filter'); break
    case 3: run(ids.claim, 1000, 'claim'); break
    default: run(ids.eob, 1000, 'eob')
  }
}
