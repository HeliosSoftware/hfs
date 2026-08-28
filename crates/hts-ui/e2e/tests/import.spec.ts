import { expect, test } from "@playwright/test";

// Phase 2 Slice F: standalone Import page (§7.7). Complements
// tests/import.rs (the Rust in-process ring against a mocked upstream)
// with browser-level coverage against the real hts backend booted by
// e2e/boot.mjs. Four describes:
//
//   - describe A walks the page shell (heading, textarea, submit, nav
//     placement, topbar chrome).
//   - describe B walks the two pre-flight gates (empty paste, invalid
//     JSON). Both render an inline OperationOutcome without hitting
//     the upstream — the round-trip contract is implicitly asserted by
//     the same-request response, matching
//     `import.rs::import_pre_flight_empty_bundle_returns_outcome_without_calling_hts`.
//   - describe C walks the four `ImportResult` status variants. Success
//     and Rejected are exercised through the real hts backend (a
//     transaction bundle with a CodeSystem PUT for the ok path, a body
//     without a `resourceType` for the rejected path). PartialSuccess
//     (207) and TooLarge (413) are Rust-side only — see the `test.skip`
//     notes below and `crates/hts-ui/tests/import.rs`.
//   - describe D walks the dual-mode htmx contract + landmark a11y.
//
// Seed data required (see e2e/README.md): none. The Import page never
// reads existing content — it POSTs to /import against the empty
// SQLite DB the boot script provisions. The Rejected fixture uses a
// body HTS refuses regardless of state.

// Inline fixtures. The Rust ring (tests/import.rs) uses
// `{"resourceType":"Bundle","type":"collection","entry":[]}` because the
// mock canned the response and never inspects the body. Playwright
// hits the real hts, so the Success fixture is a transaction bundle
// with a single `CodeSystem` PUT — the smallest shape that reliably
// resolves to 200 through hts's `POST /import`.
const VALID_TRANSACTION_BUNDLE = `{
  "resourceType": "Bundle",
  "type": "transaction",
  "entry": [
    {
      "fullUrl": "urn:uuid:hts-ui-import-spec-cs-1",
      "resource": {
        "resourceType": "CodeSystem",
        "id": "hts-ui-import-spec-cs-1",
        "url": "http://example.org/hts-ui/import-spec/cs-1",
        "version": "0.0.1-e2e",
        "name": "HtsUiImportSpecCS1",
        "status": "active",
        "content": "complete",
        "concept": [
          { "code": "hello", "display": "Hello" }
        ]
      },
      "request": {
        "method": "PUT",
        "url": "CodeSystem/hts-ui-import-spec-cs-1"
      }
    }
  ]
}`;

// A well-formed JSON object that hts's `/import` cannot recognize as a
// FHIR Bundle. Mirrors the diagnostic shape emitted by the mock in
// `tests/import.rs::CannedResponse::import_rejected` ("Body is not a
// FHIR Bundle: missing resourceType"). Parses as JSON (so it clears
// the pre-flight `serde_json::from_str` gate) and is refused by HTS.
const BUNDLE_MISSING_RESOURCE_TYPE = `{
  "foo": "bar",
  "note": "no resourceType — HTS must 400 this via its shared error mapping"
}`;

test.describe("HTS Import page shell (§7.7)", () => {
  test("GET /ui/hts/import responds 200 and renders the h1 heading", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/import");
    expect(response?.status(), "import route must respond 200").toBe(200);
    await expect(
      page.getByRole("heading", {
        name: /Import terminology/i,
        level: 1,
      }),
    ).toBeVisible();
  });

  test("paste-mode textarea renders with an accessible label", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    // The template wires a real `<label for="hts-import-bundle">` plus
    // an `aria-label` fallback (see partials/hts-import-form.html).
    // Both resolve `hts-import-bundle-textarea-label` → "FHIR Bundle
    // (JSON)".
    const textarea = page.getByLabel(/FHIR Bundle/i).first();
    await expect(textarea).toBeVisible();
    // The textarea id is the stable hook the Rust ring keys off of,
    // and the paste-mode form's name.
    await expect(textarea).toHaveAttribute("id", "hts-import-bundle");
    await expect(textarea).toHaveAttribute("name", "bundle");
  });

  test("submit button carries the hts-import-submit copy", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    const submit = page.getByRole("button", { name: /Import/i, exact: true });
    await expect(submit).toBeVisible();
    await expect(submit).toHaveAttribute("id", "hts-import-submit");
    await expect(submit).toHaveAttribute("type", "submit");
  });

  test("nav lists Import after Concept maps and before Capability & Conformance", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    // Pluck the sidebar link labels in DOM order and compare indices.
    // (The former Operations entry this test used to anchor on was
    // removed with the Operations page; Concept maps is now the last
    // "Work" entry before the Tools section.)
    const navLinks = page
      .locator("nav")
      .getByRole("link")
      .filter({ hasText: /Home|Code|Value|Concept|Import|Capability/ });
    const names = await navLinks.allTextContents();
    const conceptMapsIdx = names.findIndex((n) => /Concept maps/i.test(n));
    const importIdx = names.findIndex((n) => /Import/i.test(n));
    const capabilityIdx = names.findIndex((n) => /Capability & Conformance/i.test(n));
    expect(conceptMapsIdx, "Concept maps must be present in nav").toBeGreaterThan(-1);
    expect(importIdx, "Import must be present in nav").toBeGreaterThan(-1);
    expect(capabilityIdx, "Capability & Conformance must be present in nav").toBeGreaterThan(-1);
    expect(
      importIdx,
      "Import must come after Concept maps in the sidebar nav",
    ).toBeGreaterThan(conceptMapsIdx);
    expect(
      capabilityIdx,
      "Capability & Conformance must come after Import in the sidebar nav",
    ).toBeGreaterThan(importIdx);
  });

  test("the topbar matches HFS's on this page too", async ({ page }) => {
    await page.goto("/ui/hts/import");
    // Chrome parity is per-page, not just on Home: the layout is shared, so
    // a regression here would show up everywhere. The "dialect: en"
    // disclosure this test used to assert was removed on 2026-08-28 — it
    // shipped non-functional and HFS has no counterpart.
    const tools = page.locator(".topbar__tools");
    await expect(tools.locator(".lang-switcher")).toHaveCount(1);
    await expect(tools.locator(".theme-toggle")).toHaveCount(1);
    await expect(tools.locator(".topbar__avatar")).toHaveText("K");
    await expect(tools.locator("details")).toHaveCount(0);
  });
});

test.describe("HTS Import pre-flight validation (§7.7)", () => {
  test("empty textarea submit renders the invalid-input OperationOutcome banner", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    // Focus the textarea to make sure any autofocus/scroll behavior
    // has settled, then submit without typing anything. The
    // `bundle_trim.is_empty()` gate in src/import.rs short-circuits
    // before UpstreamClient::import_bundle is called and swaps the
    // status region with an OperationOutcome (severity=error →
    // role="alert" per partials/hts-outcome.html).
    await page.getByLabel(/FHIR Bundle/i).first().fill("");
    await page
      .getByRole("button", { name: /Import/i, exact: true })
      .click();
    const outcome = page.getByRole("alert").first();
    await expect(outcome).toBeVisible();
    // Fluent value of `hts-import-empty-bundle-error`.
    await expect(outcome).toContainText(/Paste a JSON Bundle before submitting/i);
  });

  test("invalid JSON submit renders the invalid-json OperationOutcome banner", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    // Fill the textarea with something the second pre-flight gate
    // (serde_json::from_str) rejects. HTS must not be involved — the
    // handler returns before UpstreamClient::import_bundle. The
    // banner copy resolves from `hts-import-invalid-json-error`.
    await page.getByLabel(/FHIR Bundle/i).first().fill("not-json");
    await page
      .getByRole("button", { name: /Import/i, exact: true })
      .click();
    const outcome = page.getByRole("alert").first();
    await expect(outcome).toBeVisible();
    await expect(outcome).toContainText(/not valid JSON/i);
  });
});

test.describe("HTS Import result variants (§7.7 ImportResult enum)", () => {
  test("Success: a transaction bundle renders the ok status partial", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    await page
      .getByLabel(/FHIR Bundle/i)
      .first()
      .fill(VALID_TRANSACTION_BUNDLE);
    await page
      .getByRole("button", { name: /Import/i, exact: true })
      .click();
    // Fluent value of `hts-import-status-success` — the strongest
    // signal that hts returned 200 and the template picked the
    // success arm (see partials/hts-import-status.html).
    await expect(page.getByText(/Import complete/i)).toBeVisible({
      timeout: 10_000,
    });
    // Marker guarded by the same Rust ring's success test
    // (`import_post_200_renders_success_summary`) — a future template
    // refactor that drops it must land alongside a matched Rust-side
    // change. The V3 stepped layout (#551) replaced the styling-free
    // `hts-import-status--ok` class with a `data-` attribute; the
    // *visual* success signal is `.notice` (no modifier) + a green
    // `.tag--matched`.
    await expect(
      page.locator('[data-import-status="success"]'),
    ).toBeVisible();
    await expect(
      page.locator('[data-import-status="success"] .tag.tag--matched'),
    ).toBeVisible();
    // Success must NOT borrow the amber banner partial-success uses.
    await expect(
      page.locator('[data-import-status="success"] .notice--warn'),
    ).toHaveCount(0);
  });

  test.skip(
    "PartialSuccess: a bundle that yields 207 renders the amber warn partial",
    async () => {
      // Phase 3c should verify this against a seeded backend that
      // reliably produces `errors[]` on `POST /import`. The Rust ring
      // covers the 207 arm end-to-end via a canned mock response:
      //   crates/hts-ui/tests/import.rs::
      //     import_post_207_renders_partial_success_with_issue_list
      // which asserts the `data-import-status="partial"` marker, the
      // amber `.notice--warn` + `.tag--muted` pairing, the
      // Fluent "Import partially succeeded" title, the `<details>`
      // issue expander, and the plural-selected "2 issues" heading.
      // Reproducing that trigger through the real hts backend
      // requires a bundle that HTS accepts but reports non-fatal
      // issues on — feasible only against a seeded ValueSet / CM
      // topology that Slice F does not ship.
    },
  );

  test("Rejected: a body without resourceType renders the error status partial", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    await page
      .getByLabel(/FHIR Bundle/i)
      .first()
      .fill(BUNDLE_MISSING_RESOURCE_TYPE);
    await page
      .getByRole("button", { name: /Import/i, exact: true })
      .click();
    // Fluent value of `hts-import-status-rejected`.
    await expect(page.getByText(/Import rejected/i)).toBeVisible({
      timeout: 10_000,
    });
    // The error status partial reuses the shared OperationOutcome
    // renderer (partials/hts-outcome.html) — the marker stack below is
    // the exact combination asserted by
    // `import_post_400_renders_outcome_partial` in the Rust ring.
    await expect(
      page.locator('[data-import-status="rejected"]'),
    ).toBeVisible();
    await expect(
      page.locator('[data-import-status="rejected"] .notice.notice--warn'),
    ).toBeVisible();
    await expect(
      page.locator('[data-import-status="rejected"] .tag.tag--excluded'),
    ).toBeVisible();
    await expect(
      page.locator(".hts-outcome.hts-outcome--error"),
    ).toBeVisible();
  });

  test.skip(
    "TooLarge: a 13MB+ payload renders the split-the-Bundle guidance",
    async () => {
      // 13 MB pastes are impractical over Playwright's default
      // Chromium input path (browser process memory + WS frame
      // pressure + the fact that our webServer runs on the same box).
      // The Rust ring exercises the 413 → `.notice--warn` +
      // `hts-import-too-large-hint` path via a canned response:
      //   crates/hts-ui/tests/import.rs::
      //     import_post_413_renders_too_large_guidance
      // which asserts both the Fluent title ("Bundle too large") and
      // the split-the-Bundle hint copy.
    },
  );
});

test.describe("HTS Import a11y and dual-mode contract (§7.7)", () => {
  test("the paste textarea is associated with a <label> element", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    // The form uses a real `<label for="hts-import-bundle">` — the
    // `getByLabel` locator only resolves when the label is programmatically
    // associated (aria-label / aria-labelledby / <label for=>).
    const textarea = page.getByLabel(/FHIR Bundle/i).first();
    await expect(textarea).toBeVisible();
    // Belt-and-braces: the DOM `<label for>` value must match the
    // textarea id so screen readers announce the accessible name.
    // Post form-polish (design doc §9), the label is a shared
    // `.field__label`; we anchor on the semantic `for=` target instead
    // of the removed BEM class.
    const forAttr = await page
      .locator('label[for="hts-import-bundle"]')
      .getAttribute("for");
    expect(forAttr).toBe("hts-import-bundle");
  });

  test("the result step is a labelled polite live region", async ({ page }) => {
    await page.goto("/ui/hts/import");
    // V3 stepped layout (#551): the old `<section class="hts-import"
    // aria-labelledby="hts-import-heading">` wrapper is gone — it kept
    // the step cards from being direct children of `.content`, which is
    // what `.content > .card ~ .card` needs to space them. Step 3 is now
    // its own `.card` landmark, named by its own `<h3>`, and is still
    // the polite live region htmx swaps into (§7.7 a11y note).
    const status = page.locator("#hts-import-status");
    await expect(status).toBeAttached();
    await expect(status).toHaveAttribute("aria-live", "polite");
    await expect(
      page.getByRole("region", { name: /Result/i }),
    ).toBeVisible();
  });

  test("the page renders three numbered steps", async ({ page }) => {
    await page.goto("/ui/hts/import");
    for (const step of [/Choose source/i, /Review/i, /Result/i]) {
      await expect(
        page.getByRole("heading", { name: step, level: 3 }),
      ).toBeVisible();
    }
  });

  test("the form advertises the htmx dual-mode contract", async ({ page }) => {
    await page.goto("/ui/hts/import");
    // §7.7 nojs contract: real `<form action method>` doubled by
    // `hx-post` + `hx-target` so the page works with or without JS.
    // Assert on regexes so a future refactor (e.g. absolute vs
    // relative URLs) does not cascade this test.
    // The form IS step 1's `.card` in the V3 layout, hence `#`, not the
    // former `.hts-import__form` class (which had no CSS rule).
    const form = page.locator("form#hts-import-form");
    await expect(form).toBeVisible();
    await expect(form).toHaveAttribute("method", /post/i);
    await expect(form).toHaveAttribute("action", /\/ui\/hts\/import$/);
    await expect(form).toHaveAttribute("hx-post", /\/ui\/hts\/import$/);
    await expect(form).toHaveAttribute("hx-target", /#hts-import-status/);
    // The submit lives in step 2's card and is wired back with the HTML
    // `form=` attribute — the nojs POST depends on that association.
    await expect(page.locator("#hts-import-submit")).toHaveAttribute(
      "form",
      "hts-import-form",
    );
  });
});
