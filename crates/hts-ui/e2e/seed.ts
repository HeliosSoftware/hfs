// Playwright globalSetup: seeds the hts terminology store with the fixtures
// referenced by the browser + workbench specs. Runs AFTER boot.mjs makes
// /ui/hts respond 200 and BEFORE any test executes.
//
// A .ts file on purpose: Playwright loads globalSetup through its own CJS
// transpiler, and handing it an .mjs module hangs the runner on Node >= 23
// (require(esm) collides with the transpile hook) — the run stalls forever
// right after "WebServer available" with zero output.
//
// These fixtures are not required at server boot: boot.mjs deliberately
// keeps the SQLite empty and this
// script is the e2e harness that populates it via the well-known
// `POST /import` endpoint (see crates/hts/README.md §"Import a FHIR Bundle").
//
// Every resource carries the metadata that the browser row + detail templates
// render — `title`, `publisher`, `jurisdiction`, plus `purpose`/`copyright`
// on ValueSets and ConceptMaps — so the demo never shows em-dash placeholders
// where a plausible value would fit. See design §7.10 states matrix.

const PORT = process.env.HTS_E2E_PORT || "8090";
const IMPORT_URL = `http://127.0.0.1:${PORT}/import`;
const READY_URL = `http://127.0.0.1:${PORT}/ui/hts`;

// Small pools of plausible-but-obviously-synthetic values used to enrich the
// seed. Rotating across a handful makes browser filters (publisher facet,
// jurisdiction column) show varied rows without inventing fake medical data.
const PUBLISHERS = [
  "Helios Terminology Services",
  "Acme Health Informatics",
  "Example Terminology Consortium",
];

const JURISDICTIONS = [
  {
    coding: [{ system: "urn:iso:std:iso:3166", code: "001", display: "World" }],
  },
  {
    coding: [{ system: "urn:iso:std:iso:3166", code: "US", display: "United States of America" }],
  },
  {
    coding: [{ system: "urn:iso:std:iso:3166", code: "GB", display: "United Kingdom of Great Britain and Northern Ireland" }],
  },
  {
    coding: [{ system: "urn:iso:std:iso:3166", code: "DE", display: "Germany" }],
  },
];

// Shared copyright — synthetic, obviously non-clinical.
const COPYRIGHT_LINE =
  "\u00A9 2026 Helios Terminology Services. Sample data for demonstration purposes only.";

function pickPublisher(n) {
  return PUBLISHERS[n % PUBLISHERS.length];
}

function pickJurisdiction(n) {
  return [JURISDICTIONS[n % JURISDICTIONS.length]];
}

function fillerCodeSystem(n) {
  // Rotate status across fillers so the Status filter has variety without
  // making the primary demo resources anything other than `active`.
  let status = "active";
  if (n % 7 === 0) status = "draft";
  else if (n % 11 === 0) status = "retired";
  return {
    resourceType: "CodeSystem",
    id: `ex-cs-${n}`,
    url: `http://example.org/cs/filler-${n}`,
    version: "1.0.0",
    name: `FillerCS${n}`,
    title: `Filler Code System ${n}`,
    status,
    experimental: true,
    publisher: pickPublisher(n),
    jurisdiction: pickJurisdiction(n),
    content: "not-present",
  };
}

function buildSeedBundle() {
  const entries = [];

  // -- ex-cs-1: the workbench canary. A subsumes B via nested concept,
  //    plus designation + property on A so $lookup renders the panels.
  entries.push({
    resource: {
      resourceType: "CodeSystem",
      id: "ex-cs-1",
      url: "http://example.org/cs",
      version: "1.0.0",
      name: "ExampleCodeSystem",
      title: "Example Anatomy Code System",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[0],
      jurisdiction: [JURISDICTIONS[0]],
      description:
        "A minimal two-concept demonstration CodeSystem used by the HTS workbench canary tests.",
      content: "complete",
      count: 2,
      hierarchyMeaning: "is-a",
      property: [
        { code: "status", uri: "http://hl7.org/fhir/concept-properties#status", type: "code" },
      ],
      concept: [
        {
          code: "A",
          display: "Alpha",
          designation: [{ language: "en", value: "The Alpha" }],
          property: [{ code: "status", valueCode: "active" }],
          concept: [{ code: "B", display: "Beta" }],
        },
      ],
    },
  });

  // -- Filler code systems (ex-cs-2 .. ex-cs-31) to push the browser past
  //    the default _count=25 page and expose the Load-more button.
  for (let n = 2; n <= 31; n++) {
    entries.push({ resource: fillerCodeSystem(n) });
  }

  // -- ex-cs-source / ex-cs-target: referenced by ex-cm-1's mapping group.
  entries.push({
    resource: {
      resourceType: "CodeSystem",
      id: "ex-cs-source",
      url: "http://example.org/cs/source",
      version: "1.0.0",
      name: "ExampleSourceCS",
      title: "Example Source Terms",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[1],
      jurisdiction: [JURISDICTIONS[1]],
      content: "complete",
      count: 1,
      concept: [{ code: "A", display: "Alpha (source)" }],
    },
  });
  entries.push({
    resource: {
      resourceType: "CodeSystem",
      id: "ex-cs-target",
      url: "http://example.org/cs/target",
      version: "1.0.0",
      name: "ExampleTargetCS",
      title: "Example Target Terms",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[2],
      jurisdiction: [JURISDICTIONS[2]],
      content: "complete",
      count: 1,
      concept: [{ code: "T1", display: "Target One" }],
    },
  });

  // -- ex-cs-limbs: a large flat code system so ex-vs-1's expansion has
  //    enough concepts for the flat pager to fire on the workbench.
  const limbConcepts = [];
  for (let i = 1; i <= 60; i++) {
    limbConcepts.push({ code: `limb-${i}`, display: `Limb ${i}` });
  }
  entries.push({
    resource: {
      resourceType: "CodeSystem",
      id: "ex-cs-limbs",
      url: "http://example.org/cs/limbs",
      version: "1.0.0",
      name: "ExampleLimbsCS",
      title: "Example Limbs Anatomy",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[0],
      jurisdiction: [JURISDICTIONS[0]],
      content: "complete",
      count: 60,
      concept: limbConcepts,
    },
  });

  // -- ex-vs-1: canonical flat VS the browser + workbench specs land on.
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-1",
      url: "http://example.org/vs/limbs",
      version: "1.0.0",
      name: "ExampleLimbsVS",
      title: "Example Limbs Value Set",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[0],
      jurisdiction: [JURISDICTIONS[0]],
      purpose:
        "Enumerates the ExampleLimbsCS anatomy vocabulary so the workbench $expand pager has a meaningful page ceiling.",
      copyright: COPYRIGHT_LINE,
      immutable: false,
      compose: {
        include: [{ system: "http://example.org/cs/limbs" }],
      },
    },
  });

  // -- ex-vs-tree: a hierarchical VS pulling ex-cs-1's nested A>B tree so
  //    the tree-mode workbench test can assert role="tree".
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-tree",
      url: "http://example.org/vs/tree",
      version: "1.0.0",
      name: "ExampleTreeVS",
      title: "Example Alpha/Beta Tree",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[1],
      jurisdiction: [JURISDICTIONS[1]],
      purpose:
        "Demonstrates hierarchical expansion for the workbench tree-mode toggle by re-exposing the ex-cs-1 A > B tree.",
      copyright: COPYRIGHT_LINE,
      immutable: false,
      compose: {
        include: [{ system: "http://example.org/cs" }],
      },
    },
  });

  // -- ex-vs-batch-mixed: target ValueSet for the operations workbench
  //    $batch-validate-code demo. Composes
  //    both example CodeSystems so a single batch job exercises
  //    cross-system validation in one submission (rows sourced from
  //    ex-cs-1 AND ex-cs-source resolve against the same envelope).
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-batch-mixed",
      url: "http://example.org/vs/batch-mixed",
      version: "1.0.0",
      name: "ExampleBatchMixedVS",
      title: "Example Mixed-System Batch Envelope",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[0],
      jurisdiction: [JURISDICTIONS[0]],
      purpose:
        "Target ValueSet for the $batch-validate-code demo (§3.6): composes both example CodeSystems (ex-cs-1 + ex-cs-source) so a single batch exercises cross-CS validation.",
      copyright: COPYRIGHT_LINE,
      immutable: false,
      compose: {
        include: [
          { system: "http://example.org/cs" },
          { system: "http://example.org/cs/source" },
        ],
      },
    },
  });

  // -- ex-vs-too-costly: reuses ex-cs-limbs (60 concepts). Combined with
  //    HTS_MAX_EXPANSION_SIZE=5 in boot.mjs, its default `$expand` blows
  //    past the ceiling and HTS answers 422 with a `too-costly`
  //    OperationOutcome, so the workbench renders the banner + Raise form
  //    the value-sets spec asserts on.
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-too-costly",
      url: "http://example.org/vs/too-costly",
      version: "1.0.0",
      name: "ExampleTooCostlyVS",
      title: "Example Too-Costly Expansion",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[2],
      jurisdiction: [JURISDICTIONS[3]],
      purpose:
        "Exercises the workbench too-costly banner: a 60-concept expansion under a 5-item ceiling.",
      copyright: COPYRIGHT_LINE,
      immutable: false,
      compose: {
        include: [{ system: "http://example.org/cs/limbs" }],
      },
    },
  });

  // -- Supporting VSs referenced by the ConceptMap source/target.
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-source",
      url: "http://example.org/vs/source",
      version: "1.0.0",
      name: "ExampleSourceVS",
      title: "Example Source Terms Value Set",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[1],
      jurisdiction: [JURISDICTIONS[1]],
      purpose: "Reference envelope for the ConceptMap source scope.",
      copyright: COPYRIGHT_LINE,
      immutable: false,
      compose: { include: [{ system: "http://example.org/cs/source" }] },
    },
  });
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-target",
      url: "http://example.org/vs/target",
      version: "1.0.0",
      name: "ExampleTargetVS",
      title: "Example Target Terms Value Set",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[2],
      jurisdiction: [JURISDICTIONS[2]],
      purpose: "Reference envelope for the ConceptMap target scope.",
      copyright: COPYRIGHT_LINE,
      immutable: false,
      compose: { include: [{ system: "http://example.org/cs/target" }] },
    },
  });

  // -- ex-cm-1: the canonical CM. Forward A -> T1 with "equivalent"
  //    equivalence so the workbench forward-translate test hits a match.
  entries.push({
    resource: {
      resourceType: "ConceptMap",
      id: "ex-cm-1",
      url: "http://example.org/cm/example",
      version: "1.0.0",
      name: "ExampleCM",
      title: "Example Source-to-Target Mapping",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[0],
      jurisdiction: [JURISDICTIONS[0]],
      purpose:
        "Illustrates a single equivalent mapping (Alpha -> Target One) for the workbench forward-translate demo.",
      copyright: COPYRIGHT_LINE,
      sourceUri: "http://example.org/vs/source",
      targetUri: "http://example.org/vs/target",
      group: [
        {
          source: "http://example.org/cs/source",
          target: "http://example.org/cs/target",
          element: [
            {
              code: "A",
              display: "Alpha (source)",
              target: [
                { code: "T1", display: "Target One", equivalence: "equivalent" },
              ],
            },
          ],
        },
      ],
    },
  });

  // -- ex-cm-no-match: same shape, empty mappings so a well-formed translate
  //    request returns HTTP 200 + result=false (design §7.5 F11).
  entries.push({
    resource: {
      resourceType: "ConceptMap",
      id: "ex-cm-no-match",
      url: "http://example.org/cm/no-match",
      version: "1.0.0",
      name: "ExampleCMNoMatch",
      title: "Example Empty Mapping (No-Match Demo)",
      status: "active",
      experimental: true,
      publisher: PUBLISHERS[1],
      jurisdiction: [JURISDICTIONS[3]],
      purpose:
        "Demonstrates the well-formed no-match branch: valid mapping envelope with zero elements.",
      copyright: COPYRIGHT_LINE,
      sourceUri: "http://example.org/vs/source",
      targetUri: "http://example.org/vs/target",
      group: [
        {
          source: "http://example.org/cs/source",
          target: "http://example.org/cs/target",
          element: [],
        },
      ],
    },
  });

  return {
    resourceType: "Bundle",
    type: "collection",
    entry: entries,
  };
}

async function waitForReady(timeoutMs = 60_000) {
  const start = Date.now();
  let lastErr;
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(READY_URL);
      if (res.ok) return;
      lastErr = new Error(`readiness probe ${READY_URL} responded ${res.status}`);
    } catch (err) {
      lastErr = err;
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(
    `HTS did not become ready at ${READY_URL} within ${timeoutMs}ms: ${lastErr?.message ?? "unknown"}`,
  );
}

export default async function globalSetup() {
  // Playwright's webServer.url guarantees /ui/hts is 200 before this hook,
  // but the same UI shell responds even when the terminology backend is
  // still finishing SQLite migrations. A short belt-and-braces poll makes
  // the seed retry-safe when a developer runs the suite against a fresh DB.
  await waitForReady();

  const bundle = buildSeedBundle();
  const bundleJson = JSON.stringify(bundle);

  const res = await fetch(IMPORT_URL, {
    method: "POST",
    headers: { "Content-Type": "application/fhir+json" },
    body: bundleJson,
  });

  const bodyText = await res.text();
  if (res.status !== 200 && res.status !== 207) {
    throw new Error(
      `seed import failed: ${res.status} ${res.statusText}\n${bodyText}`,
    );
  }

  let stats;
  try {
    stats = JSON.parse(bodyText);
  } catch {
    stats = { raw: bodyText };
  }

  // eslint-disable-next-line no-console
  console.log(
    `[seed] import ${res.status} ${res.statusText}: ` +
      `CS=${stats.code_systems ?? "?"} VS=${stats.value_sets ?? "?"} ` +
      `CM=${stats.concept_maps ?? "?"} concepts=${stats.concepts ?? "?"}` +
      (Array.isArray(stats.errors) && stats.errors.length > 0
        ? ` errors=${stats.errors.length}`
        : ""),
  );

  if (Array.isArray(stats.errors) && stats.errors.length > 0) {
    // Warn but do not fail: some fillers (content=not-present) can trigger
    // non-fatal notes without breaking the fixtures the specs assert on.
    // eslint-disable-next-line no-console
    console.warn(
      `[seed] non-fatal import errors:\n  ` + stats.errors.join("\n  "),
    );
  }
}
