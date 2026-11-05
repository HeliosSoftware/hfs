# Live API Demo

This page lets you make a real FHIR API call to a live HFS server and see the response.

---

## Server Metadata (CapabilityStatement)

Every FHIR server exposes a `/metadata` endpoint that describes what the server can do. This is called the **CapabilityStatement** — it tells clients which FHIR version the server supports, what resource types are available (e.g. Patient, Observation), and which operations you can perform on them (read, search, create, etc.).

Pressing **Send Request** below will issue a read-only `GET` from your browser. No authentication is required, and no data is sent or modified on the server.

<style>
.hfs-demo {
  --hfs-accent: #4a7a4a;
  --hfs-accent-bright: #5c9a5c;
  --hfs-accent-soft: rgba(92, 154, 92, 0.12);
  --hfs-line: rgba(128, 128, 128, 0.25);
  --hfs-line-strong: rgba(128, 128, 128, 0.45);
  --hfs-mute: rgba(128, 128, 128, 0.9);
  --hfs-panel: rgba(128, 128, 128, 0.04);
  --hfs-amber: #b8860b;
  --hfs-amber-soft: rgba(184, 134, 11, 0.12);
  --hfs-error: #b94a48;
  --hfs-error-soft: rgba(185, 74, 72, 0.1);

  margin: 28px 0 36px;
  font-family: inherit;
  color: inherit;
}

.hfs-demo * { box-sizing: border-box; }

/* ---------- REQUEST BLOCK ---------- */
.hfs-request {
  position: relative;
  background: var(--hfs-panel);
  border: 1px solid var(--hfs-line);
  padding: 22px 24px 18px;
  margin-bottom: 18px;
}

.hfs-request::before,
.hfs-request::after,
.hfs-bl, .hfs-br {
  content: "";
  position: absolute;
  width: 10px;
  height: 10px;
  border: 2px solid var(--hfs-accent);
  pointer-events: none;
}
.hfs-request::before { top: -1px; left: -1px; border-right: 0; border-bottom: 0; }
.hfs-request::after  { top: -1px; right: -1px; border-left: 0; border-bottom: 0; }
.hfs-bl              { bottom: -1px; left: -1px; border-right: 0; border-top: 0; }
.hfs-br              { bottom: -1px; right: -1px; border-left: 0; border-top: 0; }

.hfs-meta {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  font-size: 0.72em;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--hfs-mute);
  margin-bottom: 16px;
  padding-bottom: 10px;
  border-bottom: 1px dashed var(--hfs-line);
}
.hfs-meta .hfs-tick { color: var(--hfs-accent); }

.hfs-req-body {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
  font-family: monospace;
}

.hfs-method {
  font-weight: 700;
  font-size: 0.85em;
  letter-spacing: 0.08em;
  background: var(--hfs-accent);
  color: #fff;
  padding: 3px 10px;
  border-radius: 2px;
}

.hfs-url {
  font-size: 0.95em;
  word-break: break-all;
  flex: 1 1 300px;
}

.hfs-hdrs {
  margin-top: 14px;
  padding-top: 12px;
  border-top: 1px dashed var(--hfs-line);
  font-family: monospace;
  font-size: 0.85em;
  color: var(--hfs-mute);
}
.hfs-hdrs b { color: inherit; opacity: 1; font-weight: 600; }

/* ---------- SEND BUTTON ---------- */
.hfs-actions {
  display: flex;
  align-items: center;
  gap: 16px;
  margin: 4px 0 20px;
  flex-wrap: wrap;
}

.hfs-send {
  font-family: inherit;
  font-size: 0.9em;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: #fff;
  background: var(--hfs-accent);
  border: 1px solid var(--hfs-accent);
  padding: 11px 22px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 12px;
  position: relative;
  overflow: hidden;
  transition: background 0.2s ease, border-color 0.2s ease;
}
.hfs-send::before {
  content: "";
  position: absolute;
  inset: 0;
  background: var(--hfs-accent-bright);
  transform: translateX(-101%);
  transition: transform 0.28s cubic-bezier(0.65, 0, 0.35, 1);
  z-index: 0;
}
.hfs-send > * { position: relative; z-index: 1; }
.hfs-send:hover:not([disabled])::before { transform: translateX(0); }
.hfs-send:hover:not([disabled]) { border-color: var(--hfs-accent-bright); }
.hfs-send:active:not([disabled]) { transform: translateY(1px); }
.hfs-send[disabled] { opacity: 0.55; cursor: progress; }
.hfs-send[disabled] .hfs-arrow { animation: hfs-blink 0.9s infinite; }

.hfs-arrow {
  font-family: monospace;
  font-weight: 400;
  transition: transform 0.2s ease;
}
.hfs-send:hover:not([disabled]) .hfs-arrow { transform: translateX(4px); }

.hfs-kbd {
  font-size: 0.75em;
  color: var(--hfs-mute);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}
.hfs-kbd kbd {
  display: inline-block;
  font-family: monospace;
  border: 1px solid var(--hfs-line-strong);
  padding: 1px 6px;
  border-radius: 2px;
  font-size: 0.95em;
  margin-right: 4px;
}

@keyframes hfs-blink { 50% { opacity: 0.2; } }

/* ---------- SUMMARY CARD ---------- */
.hfs-summary {
  display: none;
  background: var(--hfs-panel);
  border: 1px solid var(--hfs-line);
  border-left: 3px solid var(--hfs-accent-bright);
  padding: 20px 24px 22px;
  margin-bottom: 14px;
}
.hfs-summary.is-visible { display: block; animation: hfs-rise 0.4s cubic-bezier(0.2, 0.8, 0.2, 1); }
.hfs-summary.is-loading { border-left-color: var(--hfs-amber); }
.hfs-summary.is-error   { border-left-color: var(--hfs-error); background: var(--hfs-error-soft); }

@keyframes hfs-rise {
  from { opacity: 0; transform: translateY(8px); }
  to   { opacity: 1; transform: translateY(0); }
}

.hfs-sum-head {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 0.72em;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--hfs-mute);
  padding-bottom: 12px;
  margin-bottom: 16px;
  border-bottom: 1px dashed var(--hfs-line);
}
.hfs-sum-head .hfs-dot {
  width: 9px; height: 9px; border-radius: 50%;
  background: var(--hfs-accent-bright);
  box-shadow: 0 0 0 4px var(--hfs-accent-soft);
  animation: hfs-pulse 2s infinite;
  flex-shrink: 0;
}
.hfs-summary.is-loading .hfs-dot { background: var(--hfs-amber); box-shadow: 0 0 0 4px var(--hfs-amber-soft); }
.hfs-summary.is-error   .hfs-dot { background: var(--hfs-error); box-shadow: 0 0 0 4px var(--hfs-error-soft); }
@keyframes hfs-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.45; } }

.hfs-sum-head .hfs-timing {
  margin-left: auto;
  font-family: monospace;
  font-variant-numeric: tabular-nums;
  color: inherit;
  opacity: 0.85;
  font-weight: 600;
  letter-spacing: 0.04em;
}

.hfs-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
  gap: 18px 24px;
  margin-bottom: 20px;
}
.hfs-stat { border-left: 2px solid var(--hfs-accent); padding-left: 12px; }
.hfs-stat-label {
  font-size: 0.7em;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--hfs-mute);
  margin-bottom: 6px;
}
.hfs-stat-value {
  font-family: inherit;
  font-size: 1.75em;
  font-weight: 600;
  line-height: 1.1;
  font-variant-numeric: tabular-nums;
}

.hfs-prose {
  font-family: inherit;
  font-size: 1em;
  line-height: 1.6;
  max-width: 70ch;
}
.hfs-prose em {
  background: var(--hfs-accent-soft);
  font-style: normal;
  font-weight: 600;
  padding: 0 3px;
  border-radius: 2px;
}
.hfs-prose code {
  font-family: monospace;
  font-size: 0.9em;
}

/* ---------- JSON VIEWER ---------- */
.hfs-json-wrap {
  display: none;
  margin-top: 4px;
}
.hfs-json-wrap.is-visible { display: block; animation: hfs-rise 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) 0.08s both; }

.hfs-json-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: #1a1a1a;
  color: #999;
  padding: 7px 14px;
  font-size: 0.72em;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  border-top: 2px solid var(--hfs-accent-bright);
  font-family: monospace;
}
.hfs-json {
  background: #1a1a1a;
  color: #e0e0e0;
  padding: 16px 20px 20px;
  font-size: 0.85em;
  line-height: 1.6;
  max-height: 520px;
  overflow: auto;
  font-family: monospace;
  margin: 0;
}
.hfs-json::-webkit-scrollbar { width: 10px; height: 10px; }
.hfs-json::-webkit-scrollbar-thumb { background: #3a3a3a; }
.hfs-json::-webkit-scrollbar-track { background: #1a1a1a; }
</style>

<div class="hfs-demo">
  <div class="hfs-request">
    <span class="hfs-bl"></span><span class="hfs-br"></span>
    <div class="hfs-meta">
      <span><span class="hfs-tick">●</span> Request · Ready</span>
      <span>HTTP / 1.1</span>
    </div>
    <div class="hfs-req-body">
      <span class="hfs-method">GET</span>
      <span class="hfs-url">https://hfs.heliossoftware.com/metadata</span>
    </div>
    <div class="hfs-hdrs">
      <b>Accept:</b> application/fhir+json
    </div>
  </div>

  <div class="hfs-actions">
    <button class="hfs-send" id="hfs-send-btn" onclick="runFhir()">
      <span>Send Request</span>
      <span class="hfs-arrow">→</span>
    </button>
  </div>

  <div class="hfs-summary" id="hfs-summary"></div>

  <div class="hfs-json-wrap" id="hfs-json-wrap">
    <div class="hfs-json-head">
      <span>Response · application/fhir+json</span>
    </div>
    <pre class="hfs-json" id="hfs-json"></pre>
  </div>
</div>

<script>
async function runFhir() {
  const btn = document.getElementById("hfs-send-btn");
  const summary = document.getElementById("hfs-summary");
  const jsonWrap = document.getElementById("hfs-json-wrap");
  const jsonEl = document.getElementById("hfs-json");

  btn.disabled = true;
  jsonWrap.classList.remove("is-visible");
  summary.className = "hfs-summary is-loading is-visible";
  summary.innerHTML = `
    <div class="hfs-sum-head">
      <span class="hfs-dot"></span>
      <span>Transmitting · awaiting server</span>
    </div>
    <div class="hfs-prose">Opening connection to <code>hfs.heliossoftware.com</code>…</div>
  `;

  const t0 = performance.now();
  try {
    const res = await fetch("https://hfs.heliossoftware.com/metadata", {
      headers: { "Accept": "application/fhir+json" }
    });
    const json = await res.json();
    const elapsed = Math.round(performance.now() - t0);

    summary.className = "hfs-summary is-visible";
    summary.innerHTML = renderSummary(json, res.status, elapsed);

    jsonEl.innerHTML = syntaxHighlight(JSON.stringify(json, null, 2));
    jsonWrap.classList.add("is-visible");
  } catch (e) {
    summary.className = "hfs-summary is-error is-visible";
    summary.innerHTML = `
      <div class="hfs-sum-head">
        <span class="hfs-dot"></span>
        <span>Request Failed</span>
      </div>
      <div class="hfs-prose">Something went wrong: <code>${e.message}</code>. This may be a network issue — check that you can reach <code>hfs.heliossoftware.com</code> from your browser.</div>
    `;
  } finally {
    btn.disabled = false;
  }
}

function renderSummary(resource, status, elapsed) {
  if (resource.resourceType !== "CapabilityStatement") {
    return `
      <div class="hfs-sum-head">
        <span class="hfs-dot"></span>
        <span>${status} · Unexpected Resource</span>
        <span class="hfs-timing">${elapsed}ms</span>
      </div>
      <div class="hfs-prose">The server returned a <code>${resource.resourceType || "unknown"}</code> resource instead of a CapabilityStatement. The full response is shown below.</div>
    `;
  }

  const fhirVersion = resource.fhirVersion || "—";
  const publisher = resource.publisher || null;
  const restResources = resource.rest?.[0]?.resource || [];
  const resourceCount = restResources.length;

  const interactions = [...new Set(restResources.flatMap(r => r.interaction?.map(i => i.code) || []))];
  const opCount = interactions.length;

  const friendly = {
    read: "reading records",
    vread: "reading specific versions",
    search: "searching",
    "search-type": "searching",
    create: "creating new records",
    update: "updating records",
    patch: "patching records",
    delete: "deleting records",
    "history-instance": "viewing history",
    "history-type": "viewing history"
  };
  const described = [...new Set(interactions.map(i => friendly[i] || i))];

  const exampleTypes = restResources.slice(0, 4).map(r => r.type);
  const typePreview = exampleTypes.join(", ") + (resourceCount > 4 ? `, and ${resourceCount - 4} more` : "");

  const opsSentence = described.length
    ? ` Supported operations include <em>${described.join(", ")}</em>.`
    : "";

  const publisherSentence = publisher
    ? ` It is published by <em>${publisher}</em>.`
    : "";

  return `
    <div class="hfs-sum-head">
      <span class="hfs-dot"></span>
      <span>${status} OK · Response Received</span>
      <span class="hfs-timing">${elapsed}ms</span>
    </div>
    <div class="hfs-stats">
      <div class="hfs-stat">
        <div class="hfs-stat-label">FHIR Version</div>
        <div class="hfs-stat-value">${fhirVersion}</div>
      </div>
      <div class="hfs-stat">
        <div class="hfs-stat-label">Resource Types</div>
        <div class="hfs-stat-value">${resourceCount}</div>
      </div>
      <div class="hfs-stat">
        <div class="hfs-stat-label">Operations</div>
        <div class="hfs-stat-value">${opCount}</div>
      </div>
    </div>
    <div class="hfs-prose">
      The server responded with a valid <em>CapabilityStatement</em>, advertising support for <em>${resourceCount} resource types</em> (${typePreview}).${opsSentence}${publisherSentence} The complete JSON document is shown below.
    </div>
  `;
}

function syntaxHighlight(json) {
  return json.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/("(\\u[\da-fA-F]{4}|\\[^u]|[^\\"])*"(\s*:)?)/g, function (match) {
      let color = "#f0abfc";
      if (/:$/.test(match)) color = "#86efac";
      return `<span style="color:${color};">${match}</span>`;
    })
    .replace(/\b(true|false)\b/g, '<span style="color:#fcd34d;">$1</span>')
    .replace(/\b(null)\b/g, '<span style="color:#999;">$1</span>')
    .replace(/\b(-?\d+\.?\d*([eE][+-]?\d+)?)\b/g, '<span style="color:#7dd3fc;">$1</span>');
}
</script>

---

## How It Works

This page uses inline JavaScript to:

1. **Fetch** the `/metadata` endpoint with `Accept: application/fhir+json`
2. **Summarize** the response in plain English — what FHIR version, how many resource types, and what operations are available
3. **Render** the full JSON response with syntax highlighting
