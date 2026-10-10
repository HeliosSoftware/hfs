// The `insert` suite of .github/workflows/fhir-benchmark.yml: single-resource
// creates (POST [type]). HFS's own suite, not upstream's: the "Run benchmark
// suites" step uses a file in this directory in place of upstream's
// k6/<suite>.js.
//
// Why it exists: upstream crud.js interleaves 9 creates with 9 reads, 9
// updates and 9 deletes at 300 VUs, so its create rate is a fixed quarter of
// its requests/s and no create-only latency exists. This suite measures only
// creates.
//
// Load shape: constant-vus, 50 VUs for 2 minutes (gracefulStop 30s). 50 is
// above the 32-connection Postgres and MongoDB pools this workflow configures,
// so the backend is saturated, and well below crud's 300, so p95 mostly shows
// the write path rather than queueing. k6 --vus / --duration (the workflow's
// vus / duration inputs) replace this scenario, as they do upstream's pinned
// ones, so a run that sets either is not comparable.
//
// Iteration: one POST Patient (Synthea-shaped: US Core race and birth-sex
// extensions, Synthea and MRN identifiers, name, telecom, address, marital
// status, communication), then four POST Observation (vital signs) whose
// subject references that Patient. The Patient id comes from the 201 body, so
// every Observation exercises reference extraction and indexing. The resources
// are written here, so the suite does not follow BENCHMARK_REF=main.
//
// Output: there is no setup() and no read, so every request is one create and
// the k6 http_reqs rate is creates/s. The check names ('insert <type> 201')
// are deliberately not crud.js's '<type> created', which crud_residue.py
// counts. The thresholds below always pass; they exist only so k6
// --summary-export carries the per-type http_req_duration submetrics.
//
// It keeps every resource it creates, so the workflow runs it LAST, after
// search and after the search-counts.txt result-size snapshot.

import http from 'k6/http'
import { check } from 'k6'

export const options = {
  discardResponseBodies: true,
  scenarios: {
    insert: {
      executor: 'constant-vus',
      vus: 50,
      duration: '2m',
      gracefulStop: '30s',
    },
  },
  thresholds: {
    'http_req_duration{resource:Patient}': ['max>=0'],
    'http_req_duration{resource:Observation}': ['max>=0'],
  },
}

const BASE_URL = __ENV.BASE_URL
const HEADERS = {
  'Accept-Encoding': 'gzip',
  'Accept': 'application/json',
  'Content-Type': 'application/json',
  'Cache-Control': 'no-cache',
}
const OBSERVATIONS_PER_PATIENT = 4
const VITALS = [
  ['8302-2', 'Body Height', 'cm', 150],
  ['29463-7', 'Body Weight', 'kg', 60],
  ['8867-4', 'Heart rate', '/min', 60],
  ['8480-6', 'Systolic blood pressure', 'mm[Hg]', 100],
]

function patient(key) {
  return {
    resourceType: 'Patient',
    extension: [
      { url: 'http://hl7.org/fhir/us/core/StructureDefinition/us-core-race',
        extension: [
          { url: 'ombCategory', valueCoding: { system: 'urn:oid:2.16.840.1.113883.6.238', code: '2106-3', display: 'White' } },
          { url: 'text', valueString: 'White' } ] },
      { url: 'http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex', valueCode: 'F' },
    ],
    identifier: [
      { system: 'https://github.com/synthetichealth/synthea', value: key },
      { type: { coding: [{ system: 'http://terminology.hl7.org/CodeSystem/v2-0203', code: 'MR', display: 'Medical Record Number' }], text: 'Medical Record Number' },
        system: 'http://hospital.smarthealthit.org', value: key },
    ],
    name: [{ use: 'official', family: `Bench${__VU}`, given: [`Insert${__ITER}`] }],
    telecom: [{ system: 'phone', value: '555-010-0000', use: 'home' }],
    gender: 'female',
    birthDate: '1980-04-12',
    address: [{ line: ['1 Benchmark Way'], city: 'Boston', state: 'MA', country: 'US' }],
    maritalStatus: { coding: [{ system: 'http://terminology.hl7.org/CodeSystem/v3-MaritalStatus', code: 'S', display: 'Never Married' }], text: 'Never Married' },
    communication: [{ language: { coding: [{ system: 'urn:ietf:bcp:47', code: 'en-US', display: 'English' }], text: 'English' } }],
  }
}

function observation(patientId, i, now) {
  const [code, display, unit, base] = VITALS[i % VITALS.length]
  return {
    resourceType: 'Observation',
    status: 'final',
    category: [{ coding: [{ system: 'http://terminology.hl7.org/CodeSystem/observation-category', code: 'vital-signs', display: 'vital-signs' }] }],
    code: { coding: [{ system: 'http://loinc.org', code, display }], text: display },
    subject: { reference: `Patient/${patientId}` },
    effectiveDateTime: now,
    issued: now,
    valueQuantity: { value: base + (__ITER % 50), unit, system: 'http://unitsofmeasure.org', code: unit },
  }
}

export default function () {
  const key = `hfs-bench-insert-${__VU}-${__ITER}`
  const p = http.post(`${BASE_URL}/Patient`, JSON.stringify(patient(key)),
    { headers: HEADERS, responseType: 'text', tags: { resource: 'Patient' } })
  if (!check(p, { 'insert Patient 201': (r) => r.status === 201 })) return
  const id = p.json('id')
  const now = new Date().toISOString()
  for (let i = 0; i < OBSERVATIONS_PER_PATIENT; i++) {
    const o = http.post(`${BASE_URL}/Observation`, JSON.stringify(observation(id, i, now)),
      { headers: HEADERS, tags: { resource: 'Observation' } })
    check(o, { 'insert Observation 201': (r) => r.status === 201 })
  }
}
