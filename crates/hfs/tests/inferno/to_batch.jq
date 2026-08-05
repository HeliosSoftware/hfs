# Rewrite a transaction Bundle into an equivalent batch Bundle.
#
# A transaction resolves urn:uuid fullUrls into literal Type/id references
# before writing; a batch does not, and per spec must not contain
# interdependent entries. This performs that resolution client-side so the
# fixtures can be loaded into a backend that declines transactions.
#
# Deliberately mirrors `resolve_bundle_references` in
# crates/persistence/src/backends/s3/bundle.rs: only an object's `reference`
# field is rewritten, and only when it starts with `urn:uuid:`. Rewriting any
# matching string would corrupt an Identifier.value that merely looks like a
# uuid — pdex_bundle_patient_999.json contains exactly that.
#
# POST becomes PUT against the derived id, which also makes the load
# idempotent: re-running no longer creates duplicates.
def uuid_of: ltrimstr("urn:uuid:");

def resolve($map):
  walk(
    if type == "object"
       and ((.reference? // "") | type) == "string"
       and ((.reference? // "") | startswith("urn:uuid:"))
    then .reference = ($map[.reference] // .reference)
    else . end
  );

(reduce (.entry[] | select((.fullUrl // "") | startswith("urn:uuid:"))) as $e ({};
   . + { ($e.fullUrl): (($e.resource.resourceType // "Unknown") + "/" + ($e.fullUrl | uuid_of)) }
 )) as $map
| .type = "batch"
| .entry |= map(
    if ((.fullUrl // "") | startswith("urn:uuid:")) then
      (.fullUrl | uuid_of) as $id
      | (.resource.resourceType // "Unknown") as $rt
      | .resource.id = $id
      | .resource |= resolve($map)
      | .fullUrl = ($rt + "/" + $id)
      | .request = { method: "PUT", url: ($rt + "/" + $id) }
    else
      .resource |= (if . == null then . else resolve($map) end)
    end
  )
