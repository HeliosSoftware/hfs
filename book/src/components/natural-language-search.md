# Natural-Language Search

Type *"female patients over 65 with a diabetes diagnosis seen in the last year"*
and get back a FHIR search query you can read, correct, and run.

The feature is **off unless you configure it**, and an operator can make it
disappear entirely. It is designed around one rule: the language model
translates, and nothing else. It never touches your data, and it never runs a
search.

## What it does — and what it does not

The server exposes `POST /$nl-search`. It takes text and returns a **query**:

```json
{
  "supported": true,
  "target": "Patient",
  "query": "gender=female&birthdate=le1961-07-14&_has:Condition:patient:code=44054006",
  "explanation": "Female patients born on or before 1961-07-14 who have a type 2 diabetes condition recorded.",
  "caveats": ["\"over 65\" was read as born on or before 1961-07-14, relative to today."]
}
```

That query is then executed by the client through the ordinary FHIR search path
— the same one every other request uses. This is deliberate:

- **Auth, tenancy, and audit stay on the code path we already trust.** The
  translator has no privileges of its own and cannot reach storage.
- **The query is reviewable before it runs.** The UI shows it in an editable
  field; nothing is translated silently.
- **A wrong translation is visible, not mysterious.** You see the query, the
  explanation, and any assumptions the model had to make.

### What is sent to the model

| Sent | Not sent |
|------|----------|
| The text the user typed | Any patient or clinical data |
| The FHIR version in play | Any resource, ever |
| The names and types of this server's searchable parameters, from the SearchParameter registry | Credentials, tenant identifiers, URLs |

The prompt is grounded in the server's *real* search parameters, so the model
is never told about a parameter this server cannot execute. The prompt itself is
a checked-in file — `crates/rest/src/handlers/nl_search_prompt.md` — so it can
be reviewed and changed like any other code.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_NL_SEARCH_ENABLED` | `true` | Master switch. `false` removes the feature: no endpoint, no page, no mention of it anywhere in the UI. |
| `HFS_NL_SEARCH_API_KEY` | *(unset)* | LLM provider API key. Without it the feature is advertised but not functional. |
| `HFS_NL_SEARCH_MODEL` | `claude-opus-4-8` | Model used for translation. |
| `HFS_NL_SEARCH_BASE_URL` | `https://api.anthropic.com` | Provider endpoint. Point it at a proxy or a self-hosted, Anthropic-compatible endpoint. |
| `HFS_NL_SEARCH_RATE_LIMIT` | `10` | Requests allowed per window, per caller. |
| `HFS_NL_SEARCH_RATE_WINDOW_SECS` | `60` | Window size, in seconds. |
| `HFS_NL_SEARCH_DAILY_LIMIT` | `200` | Requests per caller per UTC day. |
| `HFS_NL_SEARCH_MAX_CHARS` | `500` | Longest accepted input. |

There are exactly three states:

| `ENABLED` | API key | Result |
|-----------|---------|--------|
| `false` | anything | **Gone.** The endpoint 404s, the Search page is not routed, and the sidebar says nothing about it. |
| `true` | unset | **Advertised.** The Search page explains the feature, names the variables, and links here. The visual builder still works. |
| `true` | set | **Working.** |

### Turning it on

```bash
export HFS_NL_SEARCH_API_KEY=sk-ant-...
cargo run -p helios-hfs --bin hfs
```

Then open `/ui/search`.

### Turning it off for good

```bash
export HFS_NL_SEARCH_ENABLED=false
```

Nothing is mounted, nothing is rendered, and no request can reach a provider.
This is the setting for a deployment that must never make an outbound call.

## Getting an API key

The default provider is Anthropic. Create a key at
[console.anthropic.com](https://console.anthropic.com/settings/keys) and set
`HFS_NL_SEARCH_API_KEY`. To use a different provider, a gateway, or a
self-hosted model, point `HFS_NL_SEARCH_BASE_URL` at an endpoint that speaks the
Anthropic Messages API.

## Cost

You are paying for translation, not for search. One request sends the prompt
(the instructions plus this server's search-parameter vocabulary) and the user's
sentence, and gets back a query — on the order of a few thousand input tokens and
a hundred output tokens. The vocabulary block is byte-stable across requests and
marked cacheable, so a busy server pays for it at the cached rate rather than in
full on every call.

The real cost risk is not volume — it is **someone else spending your key**. See
below.

## Abuse prevention

The API key is the operator's. An exposed natural-language box is an exposed LLM
endpoint that someone else is paying for, so the feature does not rely on the
model behaving.

**The output shape is enforced, not requested.** The model must answer through a
single forced tool call with a fixed schema. A jailbroken model cannot return
free text through this endpoint — there is nowhere for it to go.

**The server does not trust the query either.** Every generated query is parsed
and checked against the SearchParameter registry before it is returned. An
unknown resource type, an unknown parameter, or a smuggled payload fails closed
with an error; it is never handed to the caller and never executed.

**The prompt refuses off-topic work.** It scopes the model to translating search
intent, treats the user's input as *data* rather than instructions, and answers
`supported: false` to anything else — general questions, code generation,
creative writing, "ignore previous instructions". Ask it for a bubble sort and
you get a refusal, not a program. And when one input packs a legitimate search
*and* something else into the same paragraph, the model translates only the
search and notes that the rest was ignored.

Prompt instructions alone are never sufficient, which is why they are paired
with the two server-side checks above: if the model is talked into misbehaving,
the response still fails validation.

**Rate limits.** Requests are counted per authenticated user (per peer address
when authentication is disabled), within a tenant:

- a sliding window (`RATE_LIMIT` per `RATE_WINDOW_SECS`),
- a daily ceiling (`DAILY_LIMIT`) as a backstop against a slow-drip abuser who
  stays under the window,
- an input length cap (`MAX_CHARS`) so nobody can paste a novel and bill it to
  you.

Exceeding a limit returns `429` with a FHIR `OperationOutcome` (`throttled`).

## Using the endpoint directly

```bash
curl -X POST http://localhost:8080/\$nl-search \
  -H 'Content-Type: application/json' \
  -d '{"text": "observations from the last 30 days, most recent first"}'
```

An unsupported request is answered, not rejected:

```json
{
  "supported": false,
  "reason": "That is a request to write code, not a search over this server's data."
}
```
