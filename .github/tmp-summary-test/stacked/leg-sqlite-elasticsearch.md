### Elasticsearch tuning (sqlite-elasticsearch)

| setting | value |
|---|---|
| image | `elasticsearch:8.15.0` (reports @@ES_VERSION@@), single node |
| JVM heap (-Xms = -Xmx) | `1024m` |
| HFS_COMPOSITE_SYNC_MODE | `synchronous` |
| HFS_ELASTICSEARCH_WRITE_REFRESH | `wait_for` |
| HFS_ELASTICSEARCH_REFRESH_INTERVAL | `200ms` |
| HFS_COMPOSITE_SYNC_REPAIR_INTERVAL | `60` |
| index replicas | 1 (hardcoded default in HFS) — cluster **yellow** on one node is expected |
| data volume free space (start) | `173119 MB (15% used)` |
### Search latency by query shape (sqlite-elasticsearch)
| Query shape | n | median (ms) | p95 (ms) | max (ms) |
|---|---:|---:|---:|---:|
| `date Observation?date` | 4617 | 59 | 113 | 381 |
| `quantity Observation?combo-value-quantity` | 4617 | 51 | 101 | 342 |
| `quantity Observation?value-quantity` | 4617 | 48 | 99 | 201 |
| `token Observation?category` | 4617 | 38 | 80 | 484 |
| `quantity Observation?component-value-quantity` | 4617 | 45 | 78 | 298 |
| `token Observation?code` | 4617 | 31 | 57 | 167 |
| `composite Observation?code-value-quantity` | 4617 | 30 | 56 | 312 |
| `date Encounter?date` | 4617 | 30 | 53 | 335 |
| `composite Observation?combo-code-value-quantity` | 4617 | 29 | 53 | 218 |
| `composite Observation?component-code-value-quantity` | 4617 | 28 | 52 | 276 |
| `string Organization?name` | 4617 | 27 | 50 | 342 |
| `date Patient?birthdate` | 4617 | 26 | 50 | 106 |
| `token Encounter?class` | 4617 | 27 | 50 | 217 |
| `token Encounter?status` | 4617 | 26 | 49 | 223 |
| `reference Encounter?subject` | 4617 | 25 | 48 | 221 |
| `string Patient?address` | 4617 | 26 | 48 | 198 |
| `reference Observation?subject` | 4617 | 25 | 48 | 94 |
| `reference Encounter?participant` | 4617 | 26 | 48 | 202 |
| `reference MedicationRequest?subject` | 4617 | 25 | 47 | 183 |
| `reference MedicationRequest?encounter` | 4617 | 24 | 47 | 151 |
| `reference MedicationRequest?requester` | 4617 | 24 | 46 | 84 |
| `string Patient?name` | 4617 | 24 | 46 | 127 |
| `reference Observation?encounter` | 4617 | 23 | 46 | 112 |
| `reference Observation?performer` | 4617 | 23 | 45 | 104 |
| `? ??http://localhost:8084/Encounter?_count=20&_elements=id` | 1 | 15 | 15 | 15 |
| `? ??http://localhost:8084/Patient?_count=20&_elements=id` | 1 | 9 | 9 | 9 |
| `? ??http://localhost:8084/Practitioner?_count=20&_elements=id` | 1 | 4 | 4 | 4 |
## FHIR Benchmark — `main` (sqlite-elasticsearch)

| | |
|---|---|
| **Commit**  | `e0bd5cb` |
| **Backend** | `sqlite-elasticsearch` |
| **Suites**  | all |

### Leg configuration

| | |
|---|---|
| **Backend** | `sqlite-elasticsearch` |
| **ES heap** | `1g` |
| **ES sync mode** | `synchronous` |
| **Legs at once (max_parallel)** | 1 |
| **Leg timeout** | 150 min |
| **Capacity gate** | need 3072 MB, available 17404 MB (source `docker-stats`), waited 2 s |


**Batch/Transaction Import** (`POST /` Bundles): 1,000/1000 bundles, 1,627,336 entries in 684 s — **6,325 resources/s** (3.89 Bundles/s) over the 257 s import scenario, after 427 s of k6 setup() (the two seed Bundles).

**Bulk Import (`$bulk-submit`):** 1,630,685/1,630,685 resources in 533.4 s (ingest until Completed 220.1 s + search-index rebuild 313.3 s, `completed`) = **3057.2 resources/s** end to end, 7408.8 resources/s ingest only; fresh database, `defer_indexing=true`.

Search check against the converted corpus: Patient 1,000/1,000 · Observation 689,080/689,080 · `Observation?code=8302-2,29463-7` 33,473/33,473 · `Encounter?class=AMB,EMER` 65,900/65,900 ✓

**ES drain:** status=`drained` (barrier HTTP 204 after 0 s); primary_live=1632062 es_live=1632062 missing=0 needs_reindex=0.

**Host load at crud start:** loadavg 6.81, 22 containers, 15265 MB MemAvailable on the Docker host (source `docker-stats`).

**How to read this leg.** Every search is answered by Elasticsearch; the primary's own search index is not written. In `synchronous` mode a write returns only after Elasticsearch has indexed it — bundles go through a per-resource-type `_bulk` request with `refresh=wait_for` against a 200ms `refresh_interval`, so **Batch/Transaction Import/crud/insert latency on this leg includes Elasticsearch indexing plus that refresh wait**, not just the primary. Every update and delete also runs a `_delete_by_query` with a forced refresh across all of the tenant's indices. Cluster health `yellow` is expected (1 replica per index, 1 node). Compare search latency with another leg only if both imported the same entry count and the ES drain above says `drained`.

### Result-size cross-check

| Query | Total | HTTP | Seconds |
|---|---:|---:|---:|
| `Patient?_summary=count` | 1000 | 200 | 0 |
| `Observation?_summary=count` | 689080 | 200 | 0 |
| `Encounter?_summary=count` | 67692 | 200 | 0 |
| `Patient?name:contains=an&_summary=count` | 353 | 200 | 0 |
| `Observation?date=ge2020-01-01&_summary=count` | 327171 | 200 | 0 |
| `Observation?date=sa2020-01-01&_summary=count` | 326951 | 200 | 0 |
| `Observation?code=8302-2,29463-7&_summary=count` | 33473 | 200 | 0 |
| `Observation?value-quantity=gt100&_summary=count` | 101155 | 200 | 0 |
| `Observation?value-quantity=sa100&_summary=count` | 101155 | 200 | 0 |
| `Observation?code-value-quantity=http://loinc.org%7C8867-4$gt100&_summary=count` | 222 | 200 | 0 |
| `Observation?combo-code-value-quantity=8480-6$gt140&_summary=count` | 1009 | 200 | 0 |
| `Encounter?class=AMB,EMER&_summary=count` | 65900 | 200 | 0 |

_totals include 0 crud/prewarm leftovers (Patient: 0) and any Batch/Transaction Import shortfall; a composite total equal to the Observation total means the name-only fallback; seconds after a 000 row overlap the previous query (HFS keeps executing after curl's --max-time)._

**Insert:** 32,230 create requests by 50 VUs, 267/s — Patient 6,446 created (p95 249.8 ms) · Observation 25,784 created (p95 241.8 ms). HFS's own suite (`.github/scripts/fhir-bench/k6/insert.js`, load shape in its header); it ran last, after the result-size snapshot, so its resources are not in those totals.
### Results

| Suite | Throughput | p50 ms | p95 ms | p99 ms | Err% | Checks ✓/✗ |
|-------|----:|-------:|-------:|-------:|-----:|-----------:|
| crud | 995 | 295.1 | 749.2 | 974.0 | 0.0% | 248,112/0 |
| Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (resources/s; latency per Bundle) | 6,325 | 3523.2 | 10268.1 | 24381.4 | 0.0% | 1,000/0 |
| Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s) | 3,057 | n/a | n/a | n/a | 0.0% | 1,630,685/0 |
| insert | 267 | 203.3 | 243.6 | 260.3 | 0.0% | 32,230/0 |
| prewarm | 90 | 89.8 | 292.3 | 363.9 | 0.0% | 2,988/0 |
| search | 920 | 28.8 | 68.9 | 101.1 | 0.0% | 110,808/0 |

_Throughput is requests/s and latency is per request, except on the Batch/Transaction Import row (resources/s and per-Bundle latency, see the **Batch/Transaction Import** line above; k6 drives it with 20 concurrent clients); the Bulk Import row is resources/s end to end until searchable, with no per-request latency (see the **Bulk Import** line above)._

### Charts

<details><summary>Chart: sqlite-elasticsearch run phases (UTC)</summary>

**sqlite-elasticsearch: run phases (UTC)**

Each bar is one suite of this leg, from its start to its end (UTC, from host-contention.txt); gaps are setup between suites.

```mermaid
%%{init: {"gantt": {"useMaxWidth": false, "barHeight": 16, "barGap": 4, "fontSize": 11, "sectionFontSize": 11, "leftPadding": 60}, "themeVariables": {"taskBkgColor": "#0969da", "taskBorderColor": "#0969da", "taskTextColor": "#ffffff", "taskTextLightColor": "#ffffff", "doneTaskBkgColor": "#54aeff", "doneTaskBorderColor": "#54aeff", "taskTextDarkColor": "#1f2328", "gridColor": "#8c959f", "sectionBkgColor": "transparent", "altSectionBkgColor": "transparent", "sectionBkgColor2": "transparent"}, "themeCSS": ".grid .tick line{stroke:#8c959f;stroke-opacity:.6}"}}%%
gantt
    dateFormat YYYY-MM-DD HH:mm:ss
    axisFormat %H:%M
    todayMarker off
    section Suites
    prewarm :t0, 2000-01-01 22:23:52, 2000-01-01 22:24:28
    import :t1, 2000-01-01 22:24:32, 2000-01-01 22:35:59
    crud :t2, 2000-01-01 22:36:03, 2000-01-01 22:40:15
    search :t3, 2000-01-01 22:40:30, 2000-01-01 22:42:34
    insert :t4, 2000-01-01 22:42:38, 2000-01-01 22:44:41
    bulk-import :t5, 2000-01-01 22:44:45, 2000-01-01 22:54:40
```

</details>

<details><summary>Chart: sqlite-elasticsearch throughput and latency</summary>

**sqlite-elasticsearch: k6 suites throughput (requests/s)**

One bar per k6 suite, higher is better; values in the Results table above.

```mermaid
%%{init: {"xyChart": {"width": 540, "height": 260, "useMaxWidth": false}, "themeVariables": {"xyChart": {"plotColorPalette": "#2f81f7", "backgroundColor": "transparent"}}}}%%
xychart-beta
    x-axis ["crud", "insert", "prewarm", "search"]
    y-axis "requests per second" 0 --> 1200
    bar [995, 267, 90, 920]
```

**sqlite-elasticsearch: Import throughput (resources/s)**

One bar per way of loading the corpus, higher is better; values in the Results table above (batch: committed Bundle entries per second of the k6 scenario; bulk: resources until the search index covers them).

```mermaid
%%{init: {"xyChart": {"width": 400, "height": 260, "useMaxWidth": false}, "themeVariables": {"xyChart": {"plotColorPalette": "#2f81f7", "backgroundColor": "transparent"}}}}%%
xychart-beta
    x-axis ["batch import", "bulk import"]
    y-axis "resources per second" 0 --> 8000
    bar [6325, 3057]
```

**sqlite-elasticsearch: latency per suite (ms)**

Overlaid bars per suite, light to dark blue: p99 (back), p95, p50 (front); lower is better; values in the Results table above (import, per Bundle, is left out).

```mermaid
%%{init: {"xyChart": {"width": 540, "height": 260, "useMaxWidth": false}, "themeVariables": {"xyChart": {"plotColorPalette": "#54aeff, #2f81f7, #0969da", "backgroundColor": "transparent"}}}}%%
xychart-beta
    x-axis ["crud", "insert", "prewarm", "search"]
    y-axis "ms" 0 --> 1200
    bar [974, 260.3, 363.9, 101.1]
    bar [749.2, 243.6, 292.3, 68.9]
    bar [295.1, 203.3, 89.8, 28.8]
```

</details>
