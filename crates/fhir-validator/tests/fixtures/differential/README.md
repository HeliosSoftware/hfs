# Differential-testing spike (issue #427)

Adjudicates the `#423` example-corpus baselines by running the **same**
resources through both our structural engine and the **HL7 reference validator**
(`validator_cli.jar`), and diffing the outcomes into three buckets.

This directory is the spike's home for docs and (phase 2) any pinned
adjudication artifacts. The spike itself is:

| Piece | File |
|---|---|
| Comparability layer (unit-tested, no Java) | `../../differential/normalize.rs` |
| Diff test (our engine vs reference) | `../../differential.rs` |
| Reference-validator driver | `../../scripts/run_reference_validator.sh` |
| CI workflow (`workflow_dispatch`) | `.github/workflows/validator-differential.yml` |

## Why this exists

`#423` swept ~8,758 published spec examples and recorded 2,329 as failing, each
now carrying a hand-written `reason`. Two limits motivated `#427`:

1. **Hand-adjudication is error-prone.** The issue author got five claims wrong
   across ~6 clusters. A mechanical oracle scales and is auditable.
2. **The sweep cannot see false negatives.** It only detects *over*-reporting.
   A resource we validate clean produces no signal even if the reference
   validator would have flagged it. For a validator, silently accepting
   non-conformant data is the more dangerous direction.

Differential testing is the only approach that covers direction 2.

## The three buckets

| Bucket | Meaning | Action |
|---|---|---|
| `both` | genuinely-invalid published example | annotate the baseline `reason`, close out |
| `only_ours` | our **false positive** | the `#424`/`#425` class — file/fix |
| `only_theirs` | our **false negative** | highest-value discovery |

## Structural-only scoping (the crux)

Our engine (`Validator::validate_sync`) is **structural only** — cardinality,
unknown elements, JSON types, required/excluded, fixed/pattern, primitive regex,
slicing. It does **not** run FHIRPath invariants or terminology bindings in this
sweep. The reference validator runs the full stack.

A naive diff would be swamped by `invariant` and terminology (`code-invalid`,
`not-found`) findings we never compute, and those would masquerade as engine
false negatives. So the comparison is scoped to **(index-free path, structural
class)**, and terminology/invariant reference findings are **counted but
excluded** from the false-negative bucket (the `out_of_scope_theirs` tally).
They are never silently dropped — the count is always reported.

Path matching strips array indices (`Patient.name.0.family` and
`Patient.name[0].family` both collapse to `Patient.name.family`). This is a
documented coarsening: two issues on different elements of the same path match.
Precise index alignment is a phase-2 refinement.

## Running

Java-free half (the comparability layer + sampler determinism) runs in the
ordinary suite:

```
cargo test -p helios-fhir-validator --all-features --test differential
```

Full differential (needs Java 21 + network for `validator_cli.jar`):

```
crates/fhir-validator/tests/scripts/run_reference_validator.sh R4 50
cargo test -p helios-fhir-validator --all-features --test differential \
  r4_differential -- --ignored --nocapture
```

Outputs land in `target/differential/`:
`<v>.reference.json` (validator output + per-resource wall-clock),
`<v>.diff.json` (the buckets + the throughput numbers).

## Posture and phase 2

This is a **spike**, not a merge gate and not per-PR — the issue's non-goals are
explicit. `workflow_dispatch` runs it on demand to **produce the numbers**
(throughput, output shape, initial bucket sizes) the issue asks to post *before*
a full harness is scoped.

Gated on those numbers, phase 2 is:

- **Sampling** — replace the alphabetic-head sample with a mix of baseline
  entries (loads `both`/`only_ours`) and clean-validating resources (loads
  `only_theirs`), widening toward the full corpus.
- **Throughput** — if per-file JVM start dominates, switch the driver to batch
  or server mode.
- **Adjudication writer** — fold confirmed `both` findings into baseline
  `reason`s, and open issues for confirmed `only_theirs` false negatives.
- **Scheduling** — add a nightly `schedule:` trigger, offset from the other
  self-hosted suites (like `validator-conformance.yml`).
