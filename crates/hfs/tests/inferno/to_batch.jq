# Rewrite a transaction Bundle into an equivalent batch Bundle.
#
# A transaction resolves urn:uuid fullUrls into literal Type/id references
# before writing; a batch does not, and per spec must not contain
# interdependent entries. This performs that resolution client-side so the
# fixtures can be loaded into a backend that declines transactions.
#
# The goal is a load *indistinguishable* from the transaction path, so
# conformance results stay comparable across backends. That means mirroring
# what the server does — including where it deliberately does nothing.
#
# ## Only POST entries are relocated
#
# `S3Backend::process_transaction` phase 1 opens with
# `if entry.method != BundleMethod::Post { continue; }`, so it assigns ids and
# builds its reference map from POST entries alone. A `PUT Type/<id>` entry
# keeps the id it names.
#
# Relocating those as well would move `Patient/85`, `/355`, `/907`, `/908` and
# `/999` onto uuid ids — the very ids Inferno is configured to test
# (`patient_ids: 85,355,907,908,us-core-client-tests-patient`), and the ids the
# Group fixtures and 21 literal `"Patient/999"` references point at. Those
# references are not `urn:uuid:`, so nothing would rewrite them and they would
# dangle.
#
# ## References to PUT entries are deliberately left unresolved
#
# Because the server's map holds no PUT fullUrls, a reference to one stays
# `urn:uuid:…` in the stored resource — 483 such references in
# uscore_bundle_patient_355.json alone. That is issue #459, and it is why
# `Observation?patient=355` returns 0 on *every* backend.
#
# Resolving them here would hand s3-elasticsearch better data than PostgreSQL
# or SQLite get: it would pass searches the others skip, and the backends would
# stop being comparable. Fixing #459 is a server change, not a loader change.
#
# ## References resolve only against *earlier* entries
#
# SQLite, PostgreSQL and MongoDB all build the map incrementally — each entry is
# resolved against the map built so far, then contributes its own mapping:
#
#     for (idx, entry) in entries.iter_mut().enumerate() {
#         resolve_bundle_references(resource, &reference_map);   // map so far
#         let result = self.process_bundle_entry_tx(&mut tx, entry).await;
#
# So a *forward* reference — one pointing at an entry defined later in the
# bundle — is never resolved by those backends. (S3's own implementation built
# the map up-front and did resolve them, which is arguably the more spec-correct
# reading, since a FHIR transaction is not supposed to be order-dependent. It is
# also now moot: S3 no longer processes transactions at all.)
#
# This mirrors the incremental form, because the backends this data is compared
# against are the ones that still run transactions. Resolving forward references
# here would give s3-elasticsearch three references the others lack, letting it
# pass searches they skip. Whether the order-dependence is itself a defect is a
# server question, not a loader one.
#
# ## Ids come from the resource first, then the fullUrl
#
# The server preserves an existing `resource.id` on POST
# (`Some(existing) => existing.to_string()`) and generates one only when absent.
# Falling back to the fullUrl uuid keeps the rewrite deterministic, and POST
# becomes PUT so re-running the loader is idempotent rather than duplicating.

def uuid_of: ltrimstr("urn:uuid:");

# The id a POST entry is written under: its own resource.id when present,
# otherwise the uuid from its fullUrl.
def post_id: (.resource.id // (.fullUrl | uuid_of));

def is_relocatable_post:
  ((.fullUrl // "") | startswith("urn:uuid:")) and (.request.method == "POST");

# Mirrors `resolve_bundle_references` in
# crates/persistence/src/backends/s3/bundle.rs: only an object's `reference`
# field is rewritten, and only when it starts with `urn:uuid:`. Rewriting any
# matching string would corrupt an Identifier.value that merely looks like a
# uuid — pdex_bundle_patient_999.json contains exactly that.
def resolve($map):
  walk(
    if type == "object"
       and ((.reference? // "") | type) == "string"
       and ((.reference? // "") | startswith("urn:uuid:"))
    then .reference = ($map[.reference] // .reference)
    else . end
  );

# Single pass, mirroring the backends: resolve each entry against the mappings
# accumulated from entries before it, then add this entry's own mapping.
. as $bundle
| reduce ($bundle.entry // [])[] as $entry (
    {map: {}, out: []};
    .map as $m
    | ($entry | .resource |= (if . == null then . else resolve($m) end)) as $resolved
    | if ($entry | is_relocatable_post) then
        ($entry | post_id) as $id
        | ($entry.resource.resourceType // "Unknown") as $rt
        | .out += [ $resolved
                    | .resource.id = $id
                    | .fullUrl = ($rt + "/" + $id)
                    | .request = { method: "PUT", url: ($rt + "/" + $id) } ]
        | .map += { ($entry.fullUrl): ($rt + "/" + $id) }
      else
        # PUT / DELETE / GET entries keep their id, url and fullUrl exactly as
        # the server leaves them; only their references are resolved.
        .out += [$resolved]
      end
  )
| .out as $entries
| $bundle
| .type = "batch"
| .entry = $entries
