import { test, expect } from "../pages/fixtures";
import { seedTwoVersions } from "../pages/api";

// The History & Versions page (/ui/history): locate an instance, read the
// two-layer diff between versions, and — the control called out explicitly —
// the "show metadata" checkbox that folds versionId/lastUpdated churn in and
// out of the diff.

test("locating an instance loads its version rail and an adjacent diff", async ({
  history,
  request,
}) => {
  const id = await seedTwoVersions(
    request,
    "Patient",
    { name: [{ family: "HistOne" }] },
    () => ({ name: [{ family: "HistTwo" }] }),
  );

  await history.goto();
  await history.locate("Patient", id);

  // #796: the rail head is the static "Versions" label; the identity lives
  // in the path line beneath it.
  await expect(history.subject).toHaveText("Versions");
  await expect(history.path).toHaveText(`/Patient/${id}/_history`);
  await expect(history.versions).toHaveCount(2);
  await expect(history.controls).toBeVisible();
  // The default adjacent comparison shows the rename in the diff.
  await expect(history.diff).toContainText("HistTwo");
});

test("a deep link loads the history straight away", async ({ history, request }) => {
  const id = await seedTwoVersions(
    request,
    "Patient",
    { name: [{ family: "Deep" }] },
    (f) => ({ ...f, active: true }),
  );

  await history.goto({ type: "Patient", id });
  await expect(history.versions).toHaveCount(2);
});

test("the 'show metadata' checkbox re-renders the diff with the metadata flag", async ({
  page,
  history,
  request,
}) => {
  const id = await seedTwoVersions(
    request,
    "Patient",
    { name: [{ family: "MetaOff" }] },
    (f) => ({ ...f, name: [{ family: "MetaOn" }] }),
  );

  await history.goto({ type: "Patient", id });
  await expect(history.diff).toContainText("MetaOn");
  await expect(history.metadataCheckbox).not.toBeChecked();

  // Checking it re-posts the diff asking for metadata to be shown; unchecking
  // asks for it to be folded away. The flag on the wire is the observable.
  const onReq = page.waitForRequest(
    (r) => r.url().endsWith("/ui/history/diff") && r.method() === "POST",
  );
  await history.metadataCheckbox.check();
  expect((await onReq).postData() ?? "").toContain("show_metadata=true");

  const offReq = page.waitForRequest(
    (r) => r.url().endsWith("/ui/history/diff") && r.method() === "POST",
  );
  await history.metadataCheckbox.uncheck();
  expect((await offReq).postData() ?? "").toContain("show_metadata=false");
});

test("picking versions from the selects re-renders the diff", async ({ history, request }) => {
  const id = await seedTwoVersions(
    request,
    "Patient",
    { name: [{ family: "SelA" }] },
    (f) => ({ ...f, name: [{ family: "SelB" }] }),
  );
  await history.goto({ type: "Patient", id });

  // Compare v1 against itself → the identical-versions banner.
  await history.fromSelect.selectOption("1");
  await history.toSelect.selectOption("1");
  await expect(history.diff).not.toContainText("SelB", { timeout: 15_000 });
});

test("an unknown instance reports not-found, not a broken diff", async ({ history }) => {
  await history.goto();
  await history.locate("Patient", "does-not-exist-x9");
  await expect(history.diff).toContainText(/no history/i);
});
