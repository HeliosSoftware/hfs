// SQL Queries workspace (#649): a stored SQLQuery Library lists in the rail,
// its SQL decodes into the editor pane, and Run executes it over its
// depends-on ViewDefinition through $sql-run.
import { expect, test } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

test("a stored SQLQuery lists, decodes its SQL, and previews rows", async ({ page, request }) => {
  const patientId = await createResource(request, "Patient", { name: [{ family: "SqlLibE2E" }] });
  const canonical = `http://example.org/ViewDefinition/e2e-lib-${Date.now()}`;
  await createResource(request, "ViewDefinition", {
    name: "e2e_lib_patients",
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  const sql = "SELECT COUNT(*) AS n FROM v";
  const libId = await createResource(request, "Library", {
    name: "e2e_patient_count",
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
    relatedArtifact: [{ type: "depends-on", resource: canonical, label: "v" }],
    content: [
      { contentType: "application/sql", data: Buffer.from(sql).toString("base64") },
    ],
  });

  // ES composites index asynchronously: the rail, the depends-on
  // resolution, and the run preview all read through search (#596).
  await waitSearchable(request, "Library", libId);
  await waitSearchable(request, "Patient", patientId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  await expect(page.locator(`#lib-rail-list [data-type='${libId}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );
  // The SQL pane holds the decoded query, not base64.
  await expect(page.locator("textarea[name='sql']")).toContainText("SELECT COUNT(*)");

  const createNew = page.locator("a[href$='?lib=new']");
  await expect(createNew).toHaveClass(/\bbtn--primary\b/);
  await expect(createNew).not.toHaveClass(/\bbtn--accent\b/);
  await expect(createNew).toHaveCSS("height", "30px");
  await expect(createNew).toHaveCSS("padding-left", "12px");

  await page.locator("a[href*='run=1']").click();
  await expect(page.locator(".data-table")).toBeVisible();
  await expect(page.locator(".data-table th", { hasText: "n" }).first()).toBeVisible();
});

/** A minimal savable sql-query Library, named for the rail. */
function starterLibrary(name: string) {
  return {
    name,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
  };
}

// "Recently used" group (#754/#755 ticket 03): SQL Queries restores its own
// stored `last` on plain arrival, and an explicit `?lib=` deep link always
// wins over it — the same resolution order the View Definitions rail proves
// (RF1), exercised here through the Library-backed page instead.
test("restores the stored last selection on plain arrival; a deep link with ?lib= wins", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const libA = await createResource(request, "Library", starterLibrary(`zq_${stamp}_a`));
  const libB = await createResource(request, "Library", starterLibrary(`zq_${stamp}_b`));
  await Promise.all([libA, libB].map((id) => waitSearchable(request, "Library", id)));

  await page.goto(`/ui/sql/queries?lib=${libA}`);
  await page.goto("/ui/sql/queries");
  await expect(page.locator(`#lib-rail-list [data-type='${libA}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );

  // A deep link always wins over the stored last, even to a different item.
  await page.goto(`/ui/sql/queries?lib=${libB}`);
  await expect(page.locator(`#lib-rail-list [data-type='${libB}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );
});
