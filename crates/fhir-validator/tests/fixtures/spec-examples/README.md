# Official FHIR example corpus baselines

Baselines for the sweep in [`../../spec_examples.rs`](../../spec_examples.rs),
which validates every published FHIR example resource against the embedded
core schema packs.

## What a baseline is

The corpus is the set of example resources the FHIR spec publishes, vendored
at `crates/fhir/tests/data/json/<VERSION>/`. Every one of them is *supposed*
to be valid against the core spec, so an entry in `knownFailures` is usually
a **false positive in our engine**.

Usually, not always. Sweeping the corpus turned up published examples that
are genuinely invalid — R4's machine-generated `Questionnaire`s omit the
required nested `linkId`, and 145 R4B `CodeSystem`s declare
`meta.profile: shareablecodesystem` and then omit the `publisher` that
profile requires. Do not assume an entry is ours to fix.

**Read the `reason` before acting on an entry.** Every entry carries one, and
it says which of three things you are looking at:

| `reason` says | What to do |
|---|---|
| Engine bug (`#424`, `#425`, or an unfiled root cause) | Fix the engine; the entry then has to be deleted |
| Documented engine limitation | Nothing, until the limitation is lifted |
| Genuine defect in the published example | Nothing — the engine is right |

That makes these files a running, adjudicated account of where the engine and
the published corpus disagree, per FHIR version, in a form review can see:

| File | Corpus |
|---|---|
| `known-failures-r4.json` | `crates/fhir/tests/data/json/R4` |
| `known-failures-r4b.json` | `crates/fhir/tests/data/json/R4B` |
| `known-failures-r5.json` | `crates/fhir/tests/data/json/R5` |

R6 has no baseline on purpose: `crates/fhir/build.rs` wipes and re-downloads
`tests/data/json/R6` from `build.fhir.org` whenever the R6 feature is on and
the local copy is over 24 hours old, so its content is volatile and nothing
stable can be pinned to it.

## The ratchet

The test fails on divergence in **either** direction:

- a file that fails and is not in the baseline (regression, or a real bug the
  sweep just found);
- a baseline entry that now validates clean (stale — delete the entry);
- a file failing with different error kinds or a different issue count than
  recorded (one bug traded for another);
- a change in the resource count or the non-resource file list (the corpus
  moved; regenerate).

Entries may only be removed by fixing the engine, or by adjudicating the
example as genuinely invalid. Adding one is a deliberate, reviewable act — it
is a record that we and the published corpus disagree, never a way to silence
an issue. Nothing here suppresses output: the issues are still emitted, still
counted, and still printed by the sweep.

## Regenerating

Every run writes its freshly computed manifest to
`target/spec-examples/<version>.actual.json`, pass or fail. The
`Validator Conformance` workflow uploads that directory as the
**spec-example-manifests** artifact.

```bash
cargo test -p helios-fhir-validator --all-features --test spec_examples \
  -- --ignored --nocapture

# inspect the diff, then accept it
cp target/spec-examples/r4.actual.json \
   crates/fhir-validator/tests/fixtures/spec-examples/known-failures-r4.json
```

The per-entry `reason` is never *generated* by the sweep, but it is carried
across a regeneration, so the `cp` above preserves the notes rather than
wiping them. An entry whose issue count or error kinds changed loses its note
on purpose: the recorded explanation may no longer describe what the engine
reports, so it has to be re-established deliberately.

Write a `reason` for any new entry. An unexplained entry is a number nobody
can act on.

## Scope

Structural validation only (`Validator::validate_sync`): no FHIRPath
invariants, no terminology bindings. Sweeping the corpus through the deferred
effects pass is a worthwhile second tier and deliberately not attempted here —
it needs an async runtime and a terminology posture, and it would conflate
engine bugs with terminology-server availability.

Profiles named in `meta.profile` that we do ship are applied; ones we do not
ship (US Core, IHE, national IGs) are ignored rather than reported, since this
is a core-spec sweep. Profile conformance is the Inferno job's business
(issue #368).
