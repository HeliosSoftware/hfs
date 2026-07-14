/*
 * SPIKE — the one question the whole editor rests on:
 *
 *   "Given a cursor at some point inside a resource, what nodes can I add here?"
 *
 * If the FHIR Schema IR from PR #232 can answer that, the editor is buildable.
 * If it can't, no amount of UI work saves it.
 *
 * This runs against the REAL R4 pack out of feat/fhir-validator. Nothing is
 * mocked. It is deliberately written outside Rust so it tests the *IR*, not an
 * integration.
 */
const zlib = require("zlib");
const fs = require("fs");

const arr = JSON.parse(zlib.gunzipSync(fs.readFileSync(process.argv[2] + "/r4.json.gz")));
const byName = {};
arr.forEach((s) => { byName[s.name || s.id] = s; });

/* Merge a schema with its base chain (Patient -> DomainResource -> Resource).
 * The IR is cooperative, not flattened, so the editor must do this itself. */
function resolved(typeName, seen = new Set()) {
  const s = byName[typeName];
  if (!s || seen.has(typeName)) return { elements: {}, required: [] };
  seen.add(typeName);
  const baseName = s.base ? s.base.split("/").pop() : null;
  const base = baseName ? resolved(baseName, seen) : { elements: {}, required: [] };
  return {
    schema: s,
    kind: s.kind,
    elements: { ...base.elements, ...(s.elements || {}) },
    required: [...(base.required || []), ...(s.required || [])],
  };
}

/* Walk a JSON pointer-ish path into the resource, tracking the schema type. */
function typeAt(resourceType, path) {
  let type = resourceType;
  for (const step of path) {
    if (typeof step === "number") continue;   // array index: same type
    const r = resolved(type);
    const el = r.elements[step];
    if (!el) return null;
    if (el.choices) return { choice: el.choices };  // value[x] cursor
    type = el.type;
  }
  return { type };
}

/* THE function. What can be added at `path` in `resource`? */
function addable(resourceType, resource, path) {
  const at = typeAt(resourceType, path);
  if (!at || !at.type) return [];

  // The value already present at the cursor, so we can subtract what is used.
  let node = resource;
  for (const step of path) node = node?.[step];

  const r = resolved(at.type);
  const out = [];

  for (const [name, el] of Object.entries(r.elements)) {
    const present = node && node[name] !== undefined;
    const isArray = !!el.array;

    // Cardinality already consumed: a non-repeating element that is set
    // cannot be offered again.
    if (present && !isArray) continue;

    // A choice element (value[x]) is offered as a type pick, and only if no
    // sibling choice is already set.
    if (el.choices) {
      const taken = el.choices.find((c) => node && node[c] !== undefined);
      if (taken) continue;
      out.push({ name, kind: "choice", choices: el.choices });
      continue;
    }
    if (el.choiceOf) continue;  // concrete arm; offered via its choice parent

    const target = byName[el.type];
    out.push({
      name,
      kind: isArray && present ? "add-another" : "add",
      type: el.type,
      typeKind: target ? target.kind : "?",
      required: r.required.includes(name),
      binding: el.binding ? el.binding.strength + " → " + el.binding.valueSet.split("/").pop() : null,
    });
  }
  return out;
}

// ---------------------------------------------------------------------------

const patient = {
  resourceType: "Patient",
  id: "donald-duck",
  name: [{ family: "Duck", given: ["Donald"] }],
  gender: "male",
  extension: [
    {
      url: "http://hl7.org/fhir/StructureDefinition/patient-birthPlace",
      valueAddress: { city: "Duckburg" },
    },
  ],
};

function show(label, path) {
  const list = addable("Patient", patient, path);
  console.log(`\n### ${label}  (path: ${JSON.stringify(path)})`);
  if (!list.length) { console.log("   (nothing addable)"); return; }
  console.log(`   ${list.length} addable nodes; showing the interesting ones:`);
  list.slice(0, 8).forEach((e) => {
    let line = `   + ${e.name}`;
    if (e.kind === "choice") line += `  [choice: ${e.choices.join(" | ")}]`;
    else line += `  : ${e.type} (${e.typeKind})${e.kind === "add-another" ? " [repeating — add another]" : ""}`;
    if (e.required) line += "  *required*";
    if (e.binding) line += `  {${e.binding}}`;
    console.log(line);
  });
}

console.log("SPIKE: what can be added, per cursor position, from the real R4 pack");
show("at the root of a Patient", []);
show("inside Patient.name[0] (a HumanName)", ["name", 0]);
show("inside the existing extension (nested extension?)", ["extension", 0]);

// The gender element is already set and non-repeating: prove it is NOT offered.
const root = addable("Patient", patient, []);
console.log("\n### cardinality already consumed");
console.log("   gender is set and max=1 → offered again?", root.some((e) => e.name === "gender"));
console.log("   name is set but repeating   → offered again?", root.some((e) => e.name === "name"));

// The extension story, in full.
console.log("\n### the extension question");
const ext = resolved("Extension");
const valueChoices = Object.keys(ext.elements).filter((k) => k.startsWith("value"));
console.log("   Extension.elements:", Object.keys(ext.elements).filter((k) => !k.startsWith("value")).join(", "));
console.log("   value[x] arms available to a blind editor:", valueChoices.length);
console.log("   Extension.extension is recursive →", JSON.stringify(ext.elements.extension));
const concrete = arr.filter((s) => s.type === "Extension" && s.name !== "Extension");
console.log("   concrete extension definitions in the pack:", concrete.length, "  <-- the gap");
