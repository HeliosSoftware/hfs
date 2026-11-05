# Rewrite a transaction Bundle into an equivalent batch Bundle.
#
# A transaction resolves urn:uuid fullUrls into literal Type/id references
# before writing; a batch does not, and per spec must not contain
# interdependent entries. This performs that resolution client-side so the
# fixtures can be loaded into a backend that declines transactions.
#
# The goal is a load *indistinguishable* from the transaction path, so
# conformance results stay comparable across backends. That means mirroring
# what the server does. When the server's resolution changes, this file has
# to change with it — it once lagged #1931/#1939 and s3-elasticsearch silently
# loaded hundreds of dangling Patient references the other backends no longer
# had (#2121).
#
# ## Every POST and instance-PUT fullUrl is mapped
#
# SQLite, PostgreSQL and MongoDB resolve a `urn:uuid` reference to any POST or
# PUT entry's fullUrl (`BundleTransactionState::record` in
# crates/persistence/src/core/transaction.rs). A POST maps to the id it is
# written under; an instance `PUT Type/<id>` maps to the `Type/<id>` it names.
#
# Only POST entries are relocated. A `PUT Type/<id>` entry keeps the id it
# names: relocating those would move `Patient/85`, `/355`, `/907`, `/908` and
# `/999` onto uuid ids — the very ids Inferno is configured to test
# (`patient_ids: 85,355,907,908,us-core-client-tests-patient`), and the ids the
# Group fixtures and 21 literal `"Patient/999"` references point at.
#
# A conditional `PUT Type?criteria` resolves to whatever the server matches,
# which cannot be known client-side; its fullUrl is left unmapped. No fixture
# uses one today.
#
# ## Resolution does not depend on entry order
#
# The server pins forward-referenced fullUrls before any entry executes
# (`pin_forward_references`), so a reference resolves whether its target comes
# before or after it in the bundle. The map is therefore built from every entry
# first, then applied to every entry.
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

# `Type/id` for an instance PUT; null for a conditional or malformed one.
def instance_put_target:
  if .request.method == "PUT"
     and ((.request.url // "") | test("^[A-Za-z]+/[A-Za-z0-9\\-.]{1,64}$"))
  then .request.url
  else null end;

# Mirrors `BundleTransactionState::resolve`: only an object's `reference`
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

. as $bundle
| (reduce ($bundle.entry // [])[] as $entry ({};
    if ($entry.fullUrl // null) == null then .
    elif ($entry | is_relocatable_post) then
      . + { ($entry.fullUrl):
              (($entry.resource.resourceType // "Unknown") + "/" + ($entry | post_id)) }
    elif ($entry | instance_put_target) != null then
      . + { ($entry.fullUrl): ($entry | instance_put_target) }
    else . end
  )) as $map
| .type = "batch"
| .entry = [ ($bundle.entry // [])[]
    | .resource |= (if . == null then . else resolve($map) end)
    | if is_relocatable_post then
        post_id as $id
        | (.resource.resourceType // "Unknown") as $rt
        | .resource.id = $id
        | .fullUrl = ($rt + "/" + $id)
        | .request = { method: "PUT", url: ($rt + "/" + $id) }
      else
        # PUT / DELETE / GET entries keep their id, url and fullUrl exactly as
        # the server leaves them; only their references are resolved.
        .
      end
  ]
