# HL7 FHIR MCP Server — design & requirements (ADR)

**Status:** Design spike / requirements — for review before building
**Scope:** the umbrella design for a new `helios-mcp` crate + `mcp` binary that
exposes HFS's FHIR capabilities as Model Context Protocol tools, resources, and
prompts.
**Owner:** Angela
**Date:** 2026-07-08

This is the "phase 1" deliverable the issue calls for: study what's out there,
understand what the leading FHIR-MCP servers do, and write a plan — the design
decisions and requirements — *before* writing the crate. It ends with a
recommended build order and the open questions to settle first.

---

## 1. Goal

Make the Helios FHIR Server directly usable by LLM agents (Claude Desktop/Code,
IDE assistants, other MCP clients) without those agents hand-rolling FHIR REST
calls, FHIRPath, or SQL-on-FHIR. The MCP server is a **thin bridge** from MCP's
JSON-RPC primitives to the existing HFS crates — not a reimplementation of FHIR
logic.

Non-goals for the first releases: being a general FHIR gateway for non-HFS
servers, a chat UI, or a clinical-decision engine. It brokers *our* server's
capabilities to agents, safely.

---

## 2. MCP in one paragraph

MCP is an open JSON-RPC 2.0 protocol standardizing how apps expose context and
actions to LLMs. A server offers three primitives: **tools** (model-invokable
functions with JSON-Schema inputs), **resources** (readable, URI-addressed
context the client can attach), and **prompts** (reusable templated workflows).
Standard transports are **stdio** (local subprocess — how Claude Desktop/Code
launch a server) and **streamable HTTP** (remote/multi-user, with SSE for
server→client streaming; it supersedes the older HTTP+SSE transport). The
current spec revisions are `2024-11-05`, `2025-03-26`, and `2025-06-18`.

---

## 3. Landscape — what the leading FHIR-MCP servers do

Surveyed July 2026. Takeaway: the category is young (most launched 2025, several
labelled alpha), converging on a small CRUDS tool set plus terminology, with the
serious ones adding auth, audit, and token-efficiency. HFS can leapfrog on
safety and analytics (SQL-on-FHIR, FHIRPath) because those crates already exist.

| Project | Lang | Transports | Tool surface | Auth | Notable |
|---|---|---|---|---|---|
| **Aidbox MCP** (Health Samurai) | Clojure (in Aidbox) | Streamable HTTP + legacy HTTP+SSE (`/mcp`, `/sse`) | `search`, `read`, `create`, `update`, `patch`, `conditional-update/patch`, `delete`, `validate` | Aidbox `AccessPolicy` + `Client` + Bearer token; `Origin` header validation | Alpha (v2505, May 2025). Session via `Mcp-Session-Id`. Model gets FHIR validation feedback and self-corrects. No MCP-level tenancy — leans on AccessPolicy. |
| **fhir-mcp-server** (The Momentum) | Python | stdio + HTTP bridge | Full CRUD | server-config | Works against Medplum/HAPI/Firely/Azure; demoed with Claude Desktop. |
| **xSoVx/fhir-mcp** | Node/TS | stdio + HTTP bridge | `fhir.capabilities/search/read/create/update`, `terminology.lookup/expand/translate` | SMART-on-FHIR (auth-code + PKCE), client-credentials, break-glass | The "security-hardened" reference: PHI redaction engine (safe/trusted modes), FHIR `AuditEvent` emission, field-selection + pagination for token efficiency, OWASP headers, rate limiting, RBAC. |
| **Medplum MCP** | TS | (Medplum-hosted) | CRUDS on FHIR | Medplum auth | Vendor-integrated. |
| **flexpa/mcp-fhir** | TS | stdio | read/search-oriented | — | Minimal reference implementation. |

**What we learn (and adopt):**

- **A small, well-named CRUDS tool set is the table stakes** — every server has
  `search`/`read`/`create`/`update`/`delete`. HFS matches this and adds
  `history`, `capabilities`, `fhirpath_evaluate`, `sof_run_view` (analytics),
  and terminology — a **wider, more analytical surface** than the CRUD-only
  crowd, which is our differentiator.
- **Streamable HTTP is the modern remote transport; stdio is mandatory for local
  agents.** Ship both. Don't build the deprecated HTTP+SSE unless a specific
  client needs it.
- **Token efficiency is a first-class design concern**, not an afterthought
  (xSoVx: field selection + pagination). FHIR resources are large; naive tools
  blow the context window. HFS must default to summaries/`_elements` and return
  resource URIs the client can fetch on demand.
- **Safety is what separates a toy from a product**: write-gating, auth-scoped
  tool availability, and audit on every call. HFS already has `helios-auth`
  (SMART scopes) and `helios-audit` (FHIR `AuditEvent`, IHE BALP) — we plug into
  them rather than reinvent PHI/audit like xSoVx had to.
- **Model self-correction loop**: Aidbox returns validation feedback so the model
  fixes its own malformed writes. Our tool errors should carry the
  `OperationOutcome` detail so the agent can act on it.

---

## 4. Key decisions (ADR)

### 4.1 SDK: use the official Rust SDK (`rmcp`), don't hand-roll

**Decision:** depend on [`rmcp`](https://crates.io/crates/rmcp) (the official
`modelcontextprotocol/rust-sdk`).

**Why:** it's mature and production-ready (v0.12 at time of writing, on
crates.io, ~3.6k★), Rust **edition 2024** (matches our workspace / MSRV 1.90),
**Apache-2.0** (compatible with our MIT), and ships exactly what we need:
- a `#[tool]` macro for typed tools (JSON-Schema derived from Rust types),
- tools / resources / prompts, plus sampling, logging, completions,
  subscriptions, elicitation,
- both transports we want: `stdio` (server) + `StreamableHttpService` (server),
  and a pluggable `Transport` trait,
- optional OAuth 2.0 (`auth` feature).

Hand-rolling JSON-RPC 2.0 + the transport/session/capability handshake is weeks
of undifferentiated work the SDK already maintains and tests. We reserve the
right to fall back to a minimal hand-rolled layer only if `rmcp` proves
license- or API-incompatible during the spike.

**Action item for the spike:** vendor a throwaway `rmcp` "hello tool" over stdio
and confirm the `#[tool]` ergonomics + JSON-Schema output against Claude
Desktop/Code before committing.

### 4.2 Transports: stdio first, then streamable HTTP

- **stdio** — the MVP transport. Local, single-user, launched as a subprocess by
  the agent. No network auth; the process boundary is the trust boundary.
- **streamable HTTP** — phase 3. Remote/multi-user; reuses the `helios-rest`
  auth/audit middleware story (bearer token → `helios-auth` principal). Session
  via `Mcp-Session-Id`; validate `Origin` for browser-originated calls (Aidbox's
  lesson). Skip the deprecated HTTP+SSE transport.

### 4.3 Share a service layer — don't fork FHIR logic

This is the most important architectural decision. Today the FHIR REST handlers
in `crates/rest/src/handlers/` orchestrate the persistence traits
(`ResourceStorage`, `SearchProvider`, `VersionedStorage`, `TransactionProvider`)
directly. If MCP tools also call those traits directly, the two surfaces will
drift (search-param parsing, `_include` handling, OperationOutcome shaping,
tenant checks).

**Decision:** extract a **version-agnostic FHIR service layer** beneath the Axum
handlers that both REST and MCP call. The REST handler becomes "parse HTTP →
call service → render HTTP"; the MCP tool becomes "parse tool args → call
service → shape for LLM". The service owns: tenant/version resolution, the
search/read/history/transaction orchestration, and canonical `OperationOutcome`
production.

**Pragmatics / phasing of the refactor:** a full extraction is large. To not
block the MVP, phase it:
1. MVP: MCP calls the persistence traits + `helios-fhirpath`/`helios-sof`
   directly through a **small internal `mcp::service` module** that mirrors the
   REST handlers' logic for the four read tools. Mark the duplicated bits with a
   `// TODO(shared-service)` so drift is visible.
2. Follow-up PR: lift the shared orchestration into a `helios-rest::service`
   (or a new `helios-fhir-service` crate) that both call. Track as its own issue.

This keeps the MVP small while committing to the no-drift end state.

### 4.4 AuthN / AuthZ

- **stdio (local):** single-user. The principal/tenant/scopes come from **config
  and env** (`HFS_MCP_*`, and the existing `HFS_AUTH_*`/`HFS_DEFAULT_TENANT`),
  not from a per-call token. The launching user *is* the authorization. Default
  to a read-only, single-tenant principal.
- **HTTP (remote):** reuse `helios-auth`. Bearer token on the MCP HTTP endpoint
  → validate via JWKS → `Principal` with SMART scopes → per-tenant. Tool
  availability and write-gating derive from the caller's scopes: a token without
  a write scope never sees the write tools advertised.
- **Scope → tool mapping:** `system/*.rs` (read/search) unlocks the read tools;
  `system/*.c/u/d` unlock the corresponding write tools *and* require
  `HFS_MCP_ALLOW_WRITE=true`. Cross-tenant/admin tools (if any) require a
  system-context scope, mirroring the console-admin tier we already built.

### 4.5 Safety / write-gating

Reads default-on, writes default-off — the industry norm and the issue's
mandate.

- Writes require **both** `HFS_MCP_ALLOW_WRITE=true` **and** an authorized write
  scope. Neither alone suffices.
- A global **read-only mode** and a **per-tool allow-list**
  (`HFS_MCP_ALLOWED_TOOLS`) so a deployment can expose, say, only
  `fhir_search` + `sof_run_view`.
- **Every tool call is audited** via `helios-audit` (FHIR `AuditEvent`, IHE
  BALP) with the resolved principal + tenant + tool name + arguments summary +
  outcome. This is a hard requirement, not optional.
- **Destructive-op confirmation:** `fhir_delete` (and bulk-ish operations)
  should require an explicit confirmation argument (e.g. `confirm: true`) so a
  model can't delete on a stray call.
- **PHI:** unlike xSoVx we do *not* build a redaction ML engine in v1 — HFS's
  guardrails are tenant isolation + SMART scopes + audit. An optional
  field-masking pass is noted as a future consideration, not a v1 requirement.

### 4.6 Result shaping for LLMs (token budget)

FHIR bundles are huge; this is a correctness concern for agents, not a nicety.

- Default search/read responses to a **summary** projection: `_summary=true` or
  a curated `_elements` set, with the full resource available via its
  `fhir://{type}/{id}` resource URI the client can fetch on demand.
- **Sane paging defaults** (small `_count`, always return the next-link/cursor)
  so a tool call can't dump 10k resources into context.
- Return **counts + a compact table** for search where possible, plus the
  resource URIs, rather than always inlining full JSON.
- A per-response **byte/'"token" budget** cap (configurable) that truncates with
  an explicit "N more — narrow your query or page" note.

### 4.7 FHIR version & tenancy

- **Version:** default R4, selected by feature flags/config exactly like the
  rest of the workspace; the tool surface is version-agnostic via the existing
  enum-wrapper pattern (`SofViewDefinition`, `Sof*`, etc.).
- **Tenancy:** every persistence op takes a `TenantContext` first. The MCP server
  resolves **one tenant per session/connection** — config default
  (`HFS_DEFAULT_TENANT`) with an optional per-call override argument on tools
  where it's safe — consistent with `crates/rest/src/tenant/`. stdio is
  effectively single-tenant; HTTP derives tenant from the token claim.

### 4.8 Error semantics

Map FHIR `OperationOutcome` → MCP tool errors so the model can act: the tool
result carries `isError: true` with the `issue[].details.text` and
`issue[].code` in the content, mirroring how the REST layer already produces
OperationOutcomes. Machine codes stay stable; human text is actionable.

---

## 5. Proposed surface (tools / resources / prompts)

Mirrors the issue, ordered by phase. Names use the `fhir_*` / `sof_*` /
`terminology_*` convention (snake_case tool ids).

### Tools

| Tool | Phase | Gate | Notes |
|---|---|---|---|
| `fhir_search` | MVP | read scope | search params, `_include`/`_revinclude`, chaining, paging; summary projection by default |
| `fhir_read` | MVP | read scope | read by type+id; `vread` by version |
| `fhir_capabilities` | MVP | read scope | distilled CapabilityStatement (types, params, ops) |
| `fhirpath_evaluate` | MVP | read scope | FHIRPath over a supplied or `fhir://`-referenced resource |
| `fhir_history` | P4 | read scope | instance / type / system history |
| `sof_run_view` | P4 | read scope | run a SQL-on-FHIR ViewDefinition; JSON/NDJSON rows |
| `terminology_lookup` | P4 | read scope | `$lookup` |
| `validate_code` | P4 | read scope | `$validate-code` |
| `expand_valueset` | P4 | read scope | `$expand` |
| `fhir_create` | P5 | write scope + `ALLOW_WRITE` | audited |
| `fhir_update` | P5 | write scope + `ALLOW_WRITE` | audited |
| `fhir_patch` | P5 | write scope + `ALLOW_WRITE` | audited |
| `fhir_delete` | P5 | write scope + `ALLOW_WRITE` | audited; requires `confirm: true` |

### Resources (readable context)

- `fhir://metadata` — server CapabilityStatement.
- `fhir://{type}/{id}` — an individual resource by reference (the on-demand fetch
  target for §4.6's URI-instead-of-inline strategy).
- `fhir://search-parameters/{type}` — supported search params per type, so the
  model forms valid queries.

### Prompts (guided workflows)

- `summarize-patient` — given a patient id, gather a clinically relevant record
  set (Conditions, Medications, Observations, …) and produce a summary.
- `build-cohort` — help the user express a cohort as a ViewDefinition / search
  and preview results.

---

## 6. Configuration (`HFS_MCP_*`)

Follows existing `HFS_*` conventions; documented in a new `/work-with-mcp` skill.

| Var | Default | Purpose |
|---|---|---|
| `HFS_MCP_TRANSPORT` | `stdio` | `stdio` or `http` |
| `HFS_MCP_BIND` | `127.0.0.1:8090` | HTTP bind address (http transport) |
| `HFS_MCP_ALLOW_WRITE` | `false` | master switch for write tools |
| `HFS_MCP_ALLOWED_TOOLS` | (all read) | per-tool allow-list |
| `HFS_MCP_DEFAULT_TENANT` | `HFS_DEFAULT_TENANT` | tenant for stdio/local |
| `HFS_MCP_AUTH_MODE` | `none` (stdio) / `bearer` (http) | how sessions authenticate |
| `HFS_MCP_MAX_RESULT_TOKENS` | e.g. `8000` | result-shaping budget (§4.6) |
| `HFS_MCP_STORAGE_BACKEND` | inherits `HFS_STORAGE_BACKEND` | shares the server's store |

Plus the existing `HFS_AUTH_*`, `HFS_AUDIT_*`, and storage vars, which the MCP
binary reuses verbatim.

---

## 7. Crate & binary shape

A new `helios-mcp` workspace member shipping an `mcp` binary — mirroring how
`helios-sof` ships `sof-server` and `helios-fhirpath` ships `fhirpath-server`.
Depends on: `rmcp`, `helios-persistence`, `helios-fhirpath`, `helios-sof`,
`helios-hts`, `helios-auth`, `helios-audit`, and (for the shared service) either
`helios-rest`'s extracted service or the interim `mcp::service` module (§4.3).

```
crates/mcp/
├── Cargo.toml            # helios-mcp; [[bin]] mcp
└── src/
    ├── main.rs           # transport selection (stdio | http), config
    ├── server.rs         # rmcp server wiring: register tools/resources/prompts
    ├── service.rs        # interim shared-FHIR-service bridge (§4.3 phase 1)
    ├── tools/            # one module per tool; typed args via #[tool]
    ├── resources.rs      # fhir:// resource handlers
    ├── prompts.rs        # summarize-patient, build-cohort
    ├── auth.rs           # session → helios-auth Principal + tenant
    └── shape.rs          # result-shaping / token budget (§4.6)
```

---

## 8. Build order (phasing)

1. **Design spike + this ADR** ← *you are here*. Ratify the SDK choice and the
   shared-service boundary; do the `rmcp` hello-tool spike against Claude.
2. **Read-only MVP:** `helios-mcp` crate + `mcp` binary, **stdio**, tools
   `fhir_search` / `fhir_read` / `fhir_capabilities` / `fhirpath_evaluate`,
   tenant + version wiring, no writes. Acceptance: Claude Desktop/Code connects
   over stdio and calls all four against a running HFS store.
3. **HTTP transport + auth/audit:** streamable HTTP, `helios-auth` scope gating,
   `helios-audit` events per tool call.
4. **Analytics + terminology:** `sof_run_view`, `fhir_history`, terminology
   tools, the `fhir://` resources, and the two prompts.
5. **Gated writes:** `create`/`update`/`patch`/`delete` behind
   `HFS_MCP_ALLOW_WRITE` + scopes, fully audited, delete needs `confirm`.
6. **Shared-service extraction** (§4.3 phase 2) — its own issue; removes the
   interim duplication.

Each phase is independently shippable and testable.

---

## 9. Testing strategy

- **Tool schemas:** snapshot each tool's generated JSON-Schema so input contracts
  can't silently change.
- **Read tools end-to-end:** drive the stdio server in-process (rmcp client
  harness) against an in-memory SQLite HFS store; assert `fhir_search`/`read`/
  `capabilities`/`fhirpath_evaluate` results.
- **Auth/scope gating:** a token without write scope must not see or be able to
  call write tools; read-only mode hides them entirely.
- **Tenant isolation:** a session bound to tenant A never returns tenant B's
  data — the same isolation invariant the REST tests assert.
- **Result shaping:** a large search returns a bounded, paged, summarized result
  under the token budget with correct next-cursor.

---

## 10. Open questions to settle before/at the spike

1. **Shared service now or after MVP?** This doc recommends *interim module now,
   extraction as a follow-up*. Confirm that's acceptable vs. blocking the MVP on
   the refactor.
2. **`rmcp` version pinning & churn.** It's pre-1.0 (v0.12) and moving; pin
   exactly and budget for occasional API breaks. Acceptable?
3. **Single binary vs. subcommand.** Ship a standalone `mcp` binary, or add an
   `hfs mcp` subcommand to the existing server? (Local agents prefer a small
   standalone binary to spawn.)
4. **Does the MCP server embed its own store, or connect to a running HFS?**
   Recommendation: **embed the same storage stack** (like `hfs` does) so stdio
   works with zero extra services; HTTP deployments can point at the same DB the
   REST server uses.
5. **Prompt scope.** Are `summarize-patient` / `build-cohort` in the first
   feature release or deferred until the tool surface settles?
6. **PHI masking.** Confirm v1 relies on scopes+tenant+audit (no redaction
   engine), with masking as a later option.

---

## 11. References

- Model Context Protocol — https://modelcontextprotocol.io
- MCP specification — https://spec.modelcontextprotocol.io
- Rust SDK (`rmcp`) — https://github.com/modelcontextprotocol/rust-sdk ·
  https://crates.io/crates/rmcp
- Aidbox MCP (Health Samurai) —
  https://www.health-samurai.io/docs/aidbox/modules/other-modules/mcp
- xSoVx/fhir-mcp (security-hardened reference) — https://github.com/xSoVx/fhir-mcp
- The Momentum fhir-mcp-server — https://github.com/the-momentum/fhir-mcp-server
- flexpa/mcp-fhir — https://github.com/flexpa/mcp-fhir
- HFS crates this bridges: `helios-persistence`, `helios-rest`,
  `helios-fhirpath`, `helios-sof`, `helios-hts`, `helios-auth`, `helios-audit`.
