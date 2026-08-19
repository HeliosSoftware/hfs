// Playwright globalSetup: seeds the hts terminology store with the fixtures
// referenced by the browser + workbench specs. Runs AFTER boot.mjs makes
// /ui/hts respond 200 and BEFORE any test executes.
//
// The design (edson/docs/hts-ui-design.md §7) does not require these fixtures
// at server boot; boot.mjs deliberately keeps the SQLite empty and this
// script is the e2e harness that populates it via the well-known
// `POST /import` endpoint (see crates/hts/README.md §"Import a FHIR Bundle").
//
// The fixture roster is derived from the specs' inline documentation:
//   - code-systems.spec.ts §7.2/§7.3:  ex-cs-1 (A subsumes B, "Alpha"
//     designation, property), plus ex-cs-2..ex-cs-31 fillers so the default
//     browser page yields a Load-more.
//   - value-sets.spec.ts §7.4:         ex-vs-1 (flat, 60 concepts so pager
//     fires), ex-vs-tree (hierarchical), plus supporting CodeSystems.
//   - concept-maps.spec.ts §7.5:       ex-cm-1 (source A -> target T1,
//     equivalent), ex-cm-no-match (structurally valid, no mappings).

const PORT = process.env.HTS_E2E_PORT || "8090";
const IMPORT_URL = `http://127.0.0.1:${PORT}/import`;
const READY_URL = `http://127.0.0.1:${PORT}/ui/hts`;

function fillerCodeSystem(n) {
  return {
    resourceType: "CodeSystem",
    id: `ex-cs-${n}`,
    url: `http://example.org/cs/filler-${n}`,
    version: "1.0.0",
    name: `FillerCS${n}`,
    status: "active",
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
      status: "active",
      content: "complete",
      hierarchyMeaning: "is-a",
      property: [
        { code: "status", uri: "http://hl7.org/fhir/concept-properties#status", type: "code" },
      ],
      concept: [
        {
          code: "A",
          display: "Alpha",
          designation: [
            { language: "en", value: "The Alpha" },
          ],
          property: [
            { code: "status", valueCode: "active" },
          ],
          concept: [
            { code: "B", display: "Beta" },
          ],
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
      status: "active",
      content: "complete",
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
      status: "active",
      content: "complete",
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
      status: "active",
      content: "complete",
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
      status: "active",
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
      status: "active",
      compose: {
        include: [{ system: "http://example.org/cs" }],
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
      status: "active",
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
      status: "active",
      compose: { include: [{ system: "http://example.org/cs/source" }] },
    },
  });
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-target",
      url: "http://example.org/vs/target",
      version: "1.0.0",
      status: "active",
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
      status: "active",
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
      status: "active",
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
