// Seeds a spread of resource types via FHIR batch bundles, so the dashboard
// chart has several series worth comparing. Timestamps are backdated later by
// backdate.py (history rows are written with server time).
const BASE = "http://localhost:8080";

const PLAN = [
  ["Patient", 140, (i) => ({ resourceType: "Patient", name: [{ family: "Seed" + i, given: ["Demo"] }], gender: i % 2 ? "female" : "male", birthDate: String(1950 + (i % 60)) })],
  ["Observation", 620, (i) => ({ resourceType: "Observation", status: "final", code: { coding: [{ system: "http://loinc.org", code: "8867-4", display: "Heart rate" }] }, valueQuantity: { value: 55 + (i % 60), unit: "beats/minute" } })],
  ["Encounter", 260, (i) => ({ resourceType: "Encounter", status: "finished", class: { system: "http://terminology.hl7.org/CodeSystem/v3-ActCode", code: "AMB" } })],
  ["Condition", 95, (i) => ({ resourceType: "Condition", code: { coding: [{ system: "http://snomed.info/sct", code: "44054006", display: "Diabetes mellitus type 2" }] } })],
  ["MedicationRequest", 45, (i) => ({ resourceType: "MedicationRequest", status: "active", intent: "order", medicationCodeableConcept: { coding: [{ system: "http://www.nlm.nih.gov/research/umls/rxnorm", code: "197361", display: "Amlodipine 5 MG" }] }, subject: { reference: "Patient/example" } })],
  ["DiagnosticReport", 30, (i) => ({ resourceType: "DiagnosticReport", status: "final", code: { coding: [{ system: "http://loinc.org", code: "58410-2", display: "CBC panel" }] } })],
  ["Practitioner", 18, (i) => ({ resourceType: "Practitioner", name: [{ family: "Doc" + i }] })],
  ["Organization", 8, (i) => ({ resourceType: "Organization", name: "Org " + i })],
];

async function postBatch(entries) {
  const bundle = {
    resourceType: "Bundle",
    type: "batch",
    entry: entries.map((resource) => ({ resource, request: { method: "POST", url: resource.resourceType } })),
  };
  const res = await fetch(BASE + "/", {
    method: "POST",
    headers: { "Content-Type": "application/fhir+json" },
    body: JSON.stringify(bundle),
  });
  if (!res.ok) throw new Error("batch failed: " + res.status + " " + (await res.text()).slice(0, 200));
  const body = await res.json();
  const bad = (body.entry || []).filter((e) => !/^2/.test(e.response?.status ?? ""));
  if (bad.length) throw new Error("entries failed: " + JSON.stringify(bad[0]).slice(0, 200));
  return entries.length;
}

let total = 0;
for (const [type, count, make] of PLAN) {
  let made = 0;
  while (made < count) {
    const n = Math.min(100, count - made);
    const entries = Array.from({ length: n }, (_, k) => make(made + k));
    await postBatch(entries);
    made += n;
  }
  total += made;
  console.log(type + ": " + made);
}
console.log("seeded " + total + " resources");
