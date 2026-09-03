# HTS UI e2e ring

Playwright + axe-core browser tests for `helios-hts-ui` (`crates/hts-ui`).
Twin of `crates/ui/e2e/`, sized down: HTS has no tenants and no auth surface.

## Layout

```
crates/hts-ui/e2e/
├── boot.mjs                 spawns the `hts` binary against a throwaway SQLite DB
├── playwright.config.ts     one project (chromium) + one nojs project
├── package.json             pins @playwright/test + @axe-core/playwright
├── tests/
│   ├── dashboard.spec.ts    Phase 1 blocker smoke — page loads, nav present
│   ├── code-systems.spec.ts Slice B — browser + detail + workbench (ex-cs-*)
│   ├── value-sets.spec.ts   Slice C — browser + detail + $expand (ex-vs-*)
│   ├── concept-maps.spec.ts Slice D — browser + detail + $translate (ex-cm-*)
│   ├── operations.spec.ts   Slice E1 — Operations workbench shell + inputs
│   └── nojs/                nojs ring — same URLs, JavaScript disabled
└── README.md                (this file)
```

Each page gets one .spec.ts file next to it; the inventory is simply the
contents of `tests/`.

### Required seed fixtures

The Playwright ring assumes the following identifiers exist in the seed
data loaded on server boot. The `boot.mjs` harness does NOT create these
today; a follow-up (Slice G) wires a `hts import` fixture pass into the
webServer step. Specs that reference these ids will fail with 404 until
the seed lands:

- CodeSystems (Slice B — `code-systems.spec.ts`):
  - `ex-cs-1` … `ex-cs-31`, `url=http://example.org/cs`, `version=1.0.0`,
    active with at least two concepts `A` (display=Alpha) and `B`
    (display=Bravo) where A subsumes B.
- ValueSets (Slice C — `value-sets.spec.ts`):
  - `ex-vs-1` — `url=http://example.org/vs/limbs`, active, expansion
    with ≥ 30 concepts so the default `count=50` still triggers
    `[Load more]` under a `count=25`-forced query, OR the spec's
    `count` seed is adjusted to match.
  - `ex-vs-tree` — a small hierarchical ValueSet whose `$expand` in
    tree mode has ≥ 2 nested levels, so `role="tree"` and the
    `showing full tree {N}` label render.
  - `ex-vs-too-costly` — a ValueSet whose default `$expand` deliberately
    trips HTS's `HTS_MAX_EXPANSION_SIZE` and returns a 422
    OperationOutcome with `code=too-costly`.
- Operations workbench (Slice E1 — `operations.spec.ts`): no new seed
  data required. The shell + widened input specs work against the free
  scope (system + code text inputs); the closure and batch-validate
  run handlers are Slice E1 stubs so specs only assert on the input
  surface. Full run coverage for `$closure` and `batch-validate`
  arrives with Slice E2.
- ConceptMaps (Slice D — `concept-maps.spec.ts`):
  - `ex-cm-1` — `url=http://example.org/cm/example`, `version=1.0.0`,
    active, `sourceUri=http://example.org/vs/source`,
    `targetUri=http://example.org/vs/target`. At least one mapping
    group whose source concept `A` in `http://example.org/cs/source`
    translates to target concept `T1` in `http://example.org/cs/target`
    with equivalence `equivalent`. A reverse `$translate` targeting
    `targetCode=T1` MUST return `result=true` with at least one match
    pointing back at `A`, so the reverse-direction spec has a positive
    control if it is un-skipped later.
  - `ex-cm-no-match` — a ConceptMap whose `$translate` for any
    well-formed forward request (e.g. `code=Z`,
    `system=http://example.org/cs/source`) returns HTTP 200 with
    `result=false` and no `match` parameters. Used to prove the §7.5
    F11 neutral no-matches state renders and is NOT the error partial.

Until the seed loader ships, run the Playwright ring against a local
`hts` populated by hand (`curl -XPUT` the fixtures `seed.ts` creates) or
skip the specs that reference the ids above with `--grep-invert`.

## Prereqs

- `hts` binary built for the current workspace:
  ```
  cargo build -p helios-hts --features sqlite,R4
  ```
- Playwright browsers installed the first time:
  ```
  cd crates/hts-ui/e2e
  pnpm install --frozen-lockfile   # or npm ci
  pnpm test:install                # playwright install --with-deps chromium
  ```

## Run

```
cd crates/hts-ui/e2e
pnpm test                          # or npm test
```

The suite starts one `hts` server (Playwright `webServer`) with
`HTS_UI_ENABLED=1` pointing at a throwaway SQLite DB in `/dev/shm` (Linux) or
`%TEMP%` (Windows), tears it down at the end, and never touches your dev
database.

Override `HTS_E2E_PORT` to move the server; set `HTS_E2E_BASE_URL` to point
the tests at a server you already have running (the CI backend matrix uses
this to run one browser suite across sqlite / postgres in parallel).

## Rings

- **chromium** — the JS-enabled ring: theme toggle, htmx interactions, axe-core
  a11y checks, no-runtime-CDN invariants.
- **nojs** — Chromium with `javaScriptEnabled: false`. Every page must render
  and every action must succeed with real form POSTs. Every control is a
  real `<a>` or `<form>` first; htmx only upgrades it.

## Design references

- [.claude/skills/hts-api-skill/ui-design-map.md](../../../.claude/skills/hts-api-skill/ui-design-map.md)
  — per-operation UI surface field / fragment matrix.
