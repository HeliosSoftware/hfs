# Grupo B (Import) — Diagnosis

## Root cause

The `#hts-import-submit` button is **not** gated by a missing import JS enabler — no `import.js` (or inline script) exists under `crates/ui/assets/` or the HTS templates. The `disabled` attribute is emitted **server-side** in `partials/hts-import-form.html` when `degraded_reason` is `Some`.

On every `GET /ui/hts/import`, `import.rs::probe_degraded` calls `UpstreamClient::health()`. Against the real `hts` binary used by Playwright, that probe **fails JSON decode**: HTS `/health` emits `"uptime_seconds"` as a JSON **float** (`helios_observability::uptime::uptime_seconds()` returns `f64`), but `UpstreamHealth` in `upstream.rs` declares `uptime_seconds: u64`. Serde rejects the float → `UpstreamError::Decode` → `degraded_reason = "upstream-shape"` → submit renders with `disabled`.

Evidence from `playwright-group-a2.log`: diagnostics test on the same server reports  
`upstream 'health' at http://127.0.0.1:8090/health returned an unrecognized body: error decoding response body`. Rust ring mocks return integer `"uptime_seconds": 42`, so `cargo test -p helios-hts-ui` never catches the mismatch.

Because degradation is set at **page render**, filling the textarea (tests 188 / 232) cannot re-enable the button — Playwright waits 30 s for an enabled control that never arrives.

## Files touched

- `crates/hts-ui/src/upstream.rs`: `UpstreamHealth` struct and `uptime_pretty()` — type mismatch vs real HTS `/health` body
- `crates/hts-ui/src/import.rs`: `probe_degraded()` — propagates health failure into `degraded_reason` on GET
- `crates/hts-ui/templates/partials/hts-import-form.html`: emits `disabled` when `degraded_reason.is_some()`
- `crates/hts/src/operations/health.rs`: emits `uptime_seconds` as `f64` via observability helper
- `crates/hts-ui/e2e/tests/import.spec.ts`: four tests call `.click()` on a button that is disabled by degraded state (symptom, not cause)

## Fix strategy (pick ONE, justify)

**None of A–D is the correct primary fix.** They assume a client-side textarea enabler that is not implemented in the current tree. The disabled state is server-rendered from a failed `/health` probe.

**Recommended (upstream-side, outside A–D):** change `UpstreamHealth.uptime_seconds` to accept the float HTS actually emits (e.g. `f64`, truncating in `uptime_pretty()`). One-line type alignment fixes all four Import tests **and** the co-failing diagnostics `/health` tab assertion, without masking real degraded behavior.

If forced to pick from the listed options only:

- **Reject A** (`force: true`): bypasses intentional degraded UX; tests would POST while the UI tells the operator the backend is down.
- **Reject B** (default submit enabled + JS ring): no JS ring exists; template already defaults enabled unless degraded — B does not address the failure mode.
- **Reject C/D** (dispatch `input` / paste listeners): irrelevant while `disabled` is set at render time from `degraded_reason`.

## Exact edits

### 1. `crates/hts-ui/src/upstream.rs` — accept float uptime from real HTS

**Current:**

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamHealth {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub uptime_seconds: u64,
}
```

**Replacement:**

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamHealth {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub backend: String,
    /// HTS emits a fractional second count from `helios_observability::uptime`.
    #[serde(default)]
    pub uptime_seconds: f64,
}
```

**Current (`uptime_pretty`):**

```rust
    pub fn uptime_pretty(&self) -> String {
        let mut secs = self.uptime_seconds;
        let days = secs / 86_400;
        secs %= 86_400;
        let hours = secs / 3_600;
        secs %= 3_600;
        let mins = secs / 60;
```

**Replacement:**

```rust
    pub fn uptime_pretty(&self) -> String {
        let mut secs = self.uptime_seconds.floor() as u64;
        let days = secs / 86_400;
        secs %= 86_400;
        let hours = secs / 3_600;
        secs %= 3_600;
        let mins = secs / 60;
```

### 2. `crates/hts-ui/src/upstream.rs` — regression test (optional but recommended)

**Add after existing `UpstreamHealth` tests:**

```rust
    #[test]
    fn health_deserializes_fractional_uptime_seconds() {
        let body = r#"{"status":"ok","uptime_seconds":0.218212,"backend":"sqlite"}"#;
        let h: UpstreamHealth = serde_json::from_str(body).expect("real HTS /health shape");
        assert_eq!(h.status, "ok");
        assert!((h.uptime_seconds - 0.218212).abs() < f64::EPSILON);
    }
```

### 3. `crates/hts-ui/e2e/tests/import.spec.ts` — optional hardening (not required if edit 1 lands)

**Current (shell test ~line 100):**

```typescript
  test("submit button carries the hts-import-submit copy", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    const submit = page.getByRole("button", { name: /Import/i, exact: true });
    await expect(submit).toBeVisible();
    await expect(submit).toHaveAttribute("id", "hts-import-submit");
    await expect(submit).toHaveAttribute("type", "submit");
  });
```

**Replacement:**

```typescript
  test("submit button carries the hts-import-submit copy", async ({
    page,
  }) => {
    await page.goto("/ui/hts/import");
    const submit = page.getByRole("button", { name: /Import/i, exact: true });
    await expect(submit).toBeVisible();
    await expect(submit).toBeEnabled();
    await expect(submit).toHaveAttribute("id", "hts-import-submit");
    await expect(submit).toHaveAttribute("type", "submit");
  });
```

## Which tests each edit fixes

| Edit | Tests unblocked |
|------|-----------------|
| `upstream.rs` `uptime_seconds: f64` (+ `uptime_pretty` floor) | **149**, **169**, **188**, **232** (Import submit enabled after healthy GET); also diagnostics `/health` tab in `diagnostics.spec.ts:193` |
| Optional `health_deserializes_fractional_uptime_seconds` unit test | Prevents recurrence; no Playwright test directly |
| Optional `toBeEnabled()` shell assertion | Fails fast if `/health` decode regresses; guards **149–232** indirectly |

## Confidence & risks

**Confidence: high.** Log line matches decode failure on live loopback; template source shows only `degraded_reason` sets `disabled`; no import JS in assets; mock tests use integer uptime and pass.

**Risks of upstream fix:**

- `uptime_pretty()` callers see truncated whole seconds — acceptable for display tiles.
- Any code constructing `UpstreamHealth { uptime_seconds: n }` with integer literals still compiles (`n` coerces to `f64`).
- `Default for UpstreamHealth` (`uptime_seconds: 0`) remains valid.

**nojs project:** Unaffected — nojs has no `import.spec.ts` today (`e2e/tests/nojs/` is empty). When a nojs Import spec is added, a healthy upstream (post-fix) renders submit **without** `disabled`, matching the design doc §7.10 “full-page result” nojs contract. If upstream were truly down, nojs **should** keep submit disabled — same as chromium.

**Do not use `{ force: true }` on Import clicks** unless deliberately testing degraded-mode behavior; it would hide this class of `/health` contract drift.
