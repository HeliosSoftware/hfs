# cds-hooks-server

A [CDS Hooks 2.0.1](https://cds-hooks.hl7.org/) service for many clients, built on [`helios-cds-hooks`](../cds-hooks): a client registry, JWT validation per client, replay protection shared across instances, discovery computed per client, each client's own URL, and shared services each client enables. It ships one service, `patient-greeter` (`greeter_service.rs`), the smallest complete `CdsHooksService`: `patient-view` answered with one informational card. Real services are added as in [Adding a service](#adding-a-service); demo services live on their own branches, not here.

It is the first phase of [the server-side discussion](https://github.com/HeliosSoftware/hfs/discussions/2275) (revision 2): one binary, one YAML registry, no control panel yet.

## What it does

```
GET  /cds-services                 the caller's catalog: its enabled services, its prefetch
POST /cds-services/{id}            a hook fired; only the caller's enabled services answer
POST /cds-services/{id}/feedback   what the clinician did with the cards
GET  /c/{client}/cds-services      the same three routes at the client's own URL
POST /c/{client}/cds-services/{id}
POST /c/{client}/cds-services/{id}/feedback
GET  /health
```

Every CDS route, discovery included, resolves the caller first:

1. The `Authorization: Bearer` JWT is read. Its header must name a `kid` and an RSA or EC algorithm.
2. The `iss` claim, with the `tenant` claim when present (both read before verification, only to choose the keys), is looked up in the registry. An issuer or tenant the registry does not know is refused with 401, however valid the signature.
3. The signing key is fetched from the `jku` **the registry holds for that client**, never from the token, through `helios-auth`'s `JwksCache` (one per issuer, refreshed on an unknown `kid` at most once a minute).
4. `aud` must be one of the registry's `audience` values, `exp` must be in the future (60 s leeway), `iss` and `tenant` must match the client's.
5. `jti` must not have been seen before for that client, on any instance; a repeat is refused. The replay store is Postgres when `CDS_HOOKS_REPLAY_DATABASE_URL` is set (the table `cds_hooks_jti` is created when missing; it holds nonces and expiry times only), else this process's memory.
6. If the call names a `fhirServer` and the client has one on file, they must match; a mismatch is 403.
7. At the path form, the client the path names must be the client the token resolved to; anything else is 403. The path alone never identifies a caller: it exists for EMRs that cannot sign their discovery call (their catalog is still computed from the registry entry the token names) and for dedicated deployments routed by path.

Services are shared. A client sees in discovery only the services its entry enables, with its own prefetch templates when it overrides them, and `POST /cds-services/{id}` answers 404 for a service the client has not enabled. A request whose `hook` is not the service's hook is 400.

Logs carry the client id, the service id and the `hookInstance`; never a patient id, a card or a token.

## The client registry

`CDS_HOOKS_CLIENTS` names a YAML file; [`clients.example.yaml`](clients.example.yaml) documents every field. A client is one healthcare organization on one EMR instance: one `iss`, its `jku`, optionally its `fhir_server`, the `services` it enables, and optional `prefetch` templates per service. Two hospitals on the same vendor are two clients; a hospital's production and test systems are two clients. Two hospitals behind one cloud EMR that signs for both with one key are two clients with the same `iss` and `jku` and their own `tenant`, the optional JWT claim the spec defines for exactly this.

The registry is validated at load: unique ids, unique (`iss`, `tenant`) pairs, one `jku` per issuer, either every client of an issuer with a tenant or none, `iss` and `jku` together or not at all, `audience` required as soon as any client signs, and `anonymous_client` naming a registered client. A service a client enables that this binary does not offer is logged at startup and ignored.

Without `CDS_HOOKS_CLIENTS` the server runs with a **development registry**: one client, `dev`, that every unsigned call is, with every service enabled. The same thing is available with a real registry through `anonymous_client`, for a local EMR that does not sign yet. Both are logged as warnings at startup; neither belongs in production.

## Running

```sh
CDS_HOOKS_CLIENTS=clients.yaml CDS_HOOKS_PORT=8787 cargo run -p helios-cds-hooks-server
```

| Variable | Default | Purpose |
|---|---|---|
| `CDS_HOOKS_CLIENTS` | none (development registry) | The client registry file |
| `CDS_HOOKS_REPLAY_DATABASE_URL` | none (per-process memory) | Postgres URL of the `jti` replay store shared by every instance |
| `CDS_HOOKS_PORT` | `8787` | Listening port |
| `CDS_HOOKS_HOST` | `127.0.0.1` | Bind address |
| `CDS_HOOKS_LOG_LEVEL` | `info` | `tracing` filter |

CORS allows any origin, so the CDS Hooks sandbox can call the service from a browser.

## Adding a service

Implement `helios_cds_hooks::CdsHooksService` (see `greeter_service.rs`), add it to `server::catalog()`, and enable its id in the clients that should see it. The catalog erases the service's context type, so services of different hooks live in one list and are addressed by id.

## Tests

`cargo test -p helios-cds-hooks-server`. `tests/multiclient.rs` runs signed clients against one server with a throwaway RSA key served from an in-process JWKS endpoint: each sees its own catalog, only an enabled service answers, two tenants of one issuer are two clients, the path form agrees with the token or refuses, and unsigned, unknown-issuer, unknown-tenant, expired, mis-addressed, unknown-key, replayed, foreign-FHIR-server and wrong-path calls are refused. The key under `tests/fixtures` is for tests only. The Postgres replay store has a test that runs when `CDS_HOOKS_TEST_DATABASE_URL` names a database and is skipped otherwise.

## Not here yet

The central settings server behind the registry, the control panel, rule activation separate from service enablement, per-client rate limits, the shared PHI-free cache, the PHI cache with its controls, and the feedback store: the later phases of the discussion document.
