# Schema-driven FHIR Resource Editor — research & design

**Status:** design proposal, for review. No production code.
**Issue:** #264. **Depends on:** #232 (`helios-fhir-validator`, draft) and the architecture question in discussion #215.

---

## 1. The finding that should shape the decision

An editor and a validator want two different things from a schema.

A validator asks: *"here is a document — is it legal?"* It needs to answer that once, quickly, and it can be compiled into code that knows the answer ahead of time.

An editor asks: *"here is a cursor — what am I allowed to put here?"* That question has no document to check. It is a query **against the schema itself**, at runtime, at every node the user touches.

Discussion #215 makes an explicit architectural bet:

> *"We will not walk `StructureDefinition` snapshots at runtime. We will compile them — ahead of time, into Rust."*

That bet is sound for validation and says nothing about editing. But it has a consequence nobody has written down yet, so let us write it down:

> **A compiled validator cannot drive an editor.** `#[derive(FhirValidate)]` gives you `validate()`. It does not give you *"the 22 elements addable at this cursor, in spec order, minus the ones whose cardinality is already spent, plus the 50 `value[x]` arms if the cursor is inside an extension."* An editor needs the FHIR Schema IR **as runtime-queryable data**, not as generated control flow.

This is not an argument against codegen. It is a requirement on it: **whichever way #215 lands, the FHIR Schema IR must remain available as data at runtime.** Today's schema packs (PR #232) satisfy that. A codegen path satisfies it only if it *also* emits the IR — as a static table, an embedded pack, or a `const` structure. If the codegen path wins and drops the data representation, this editor cannot be built, and neither can any other schema-aware UI (the SearchParameter editor in #238 and the Compartment editor in #237 are already leaning the same way).

Everything below assumes only that: **a resolver from a type name to a FHIR Schema, available in-process, synchronously.** It does not depend on runtime packs specifically.

---

## 2. Does the IR actually carry an editor? (Yes — spike results)

Before designing anything, I checked whether the hard question is answerable at all. I ran the "what can I add here?" algorithm against the **real R4 pack** from `feat/fhir-validator` (`packs/fhir_schemas_r4.json.gz`), on a Patient shaped like Steve's Donald Duck example.

The IR is a flat set of **257 schemas** (192 resources, 44 complex types, 20 primitives, 1 logical). `Patient` is **4.8 KB** and carries 29 first-level elements. Elements are lean and cooperative:

```json
"name":     { "type": "HumanName", "array": true },
"gender":   { "type": "code", "binding": { "valueSet": ".../administrative-gender|4.0.1",
                                           "strength": "required" } },
"deceased": { "choices": ["deceasedBoolean", "deceasedDateTime"] },
"extension":{ "type": "Extension", "array": true }
```

The spike, per cursor position:

| Cursor | Addable nodes |
|---|---|
| root of a `Patient` | **22** — in spec order, with bindings attached |
| inside `Patient.name[0]` (a `HumanName`) | **8** — *including `extension`* |
| inside an existing extension | `id`, `extension` (recursive) |
| inside a **new, empty** extension | `url`, `extension`, and `value[x]` → **a pick from 50 types** |

And the two behaviours that make it a real editor rather than a form:

- **Cardinality already spent is excluded, for free.** `gender` is set and non-repeating → not offered again. `name` is set but `array: true` → offered as *"add another"*.
- **Extensions nest, for free.** `Extension.extension` is recursive in the IR, so Steve's *"you can have extensions of extensions, which is crazy"* falls out of the same walk with no special case. Pick `valueAddress` and the editor simply descends into `Address` (12 elements).

**Conclusion: the schema layer is sufficient.** The editor is a projection problem, not an acquisition problem. What is missing is not information — it is a *function*, and that function does not exist yet (see §4).

---

## 3. Research question 1 — how does the editor reach the schema?

**Recommendation: in-process. `helios-ui` depends on the schema crate directly. No HTTP schema surface.**

The cost argument settles it. Resolution is:

```rust
pub trait SchemaResolver: Send + Sync {
    fn resolve(&self, reference: &str) -> Option<Arc<FhirSchema>>;   // that is the whole trait
}
```

Synchronous. No I/O. A `HashMap` lookup and an `Arc` clone. `core_registry(version)` decompresses and parses the pack **once per process** behind a `OnceLock`, then hands out `Arc` clones forever. Rendering one node costs microseconds; there is nothing to amortise and nothing worth caching across a request.

An HTTP schema-projection endpoint would add a network hop, a serialization format to version, and a second thing to keep in sync — to save a call that costs less than the logging around it. Rejected.

The dependency is also cheaper than it looks. `helios-fhir-validator` pulls only `helios-fhir`, `serde`, `indexmap`, `flate2` and `regex` — and **FHIRPath is an optional feature** (`fhirpath = ["dep:helios-fhirpath"]`). So `helios-ui` can take the crate *without* it and get the schema IR and the structural pass — which is exactly what the live editing loop needs — while FHIRPath constraint evaluation stays where it already is, behind the REST layer's async effects pass.

Two consequences worth naming:

- The whole R4 pack is **1.21 MB decompressed, resident**, per version enabled. That is the price of the in-process choice. It is already being paid by `$validate` on the write path.
- **Core-pack schemas are snapshot-derived and therefore already flat.** `resolve("Patient").elements` yields all 29 children *including inherited* `id`/`meta`/`text`/`extension` — no base-chain walking needed. Base walking only becomes necessary for **tenant-uploaded differential profiles**, which are sparse. Design for the walk; enjoy not needing it on the common path.

---

## 4. Research question 3 — adding nodes (the core requirement)

This is the function the whole editor rests on, and **it does not exist in the crate today**. The engine's cooperative schema-set machinery — the code that actually knows the merged element map for a node — is private (`SchemaSet`, `add_schemas_to_set` are `pub(super)`; `mod walk` is private). A consumer cannot reach it.

So we write it. **And we write it in the schema crate, not in `helios-ui`** — because #237 and #238 want the same projection, and a form-metadata layer buried in a web crate is a layer we will regret.

Proposed shape (a new module, or a thin `helios-fhir-schema-view` crate if #215 goes the codegen way):

```rust
/// What may be added at `path` inside `document`, in spec order.
pub fn addable(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    document: &serde_json::Value,
    path: &[Step],           // Step::Field(&str) | Step::Index(usize)
) -> Vec<Addable>;

pub struct Addable {
    pub name: String,
    pub kind: AddableKind,   // Add | AddAnother | Choice { arms: Vec<String> }
    pub type_: Option<String>,
    pub required: bool,
    pub binding: Option<Binding>,
    pub refers: Option<Vec<String>>,   // scope a reference picker
}
```

The algorithm, with the traps that the spike hit and every naive implementation will hit:

1. **Resolve the type at the cursor** by walking the path, following `element.type_` for named types and the *inline* `elements` map for BackboneElements (`Patient.contact` carries its children inline; it is not a top-level schema).
2. **Filter out choice declarers.** A `value[x]` appears in `elements` as **three sibling keys**: the declarer `deceased` (carrying `choices: [...]`), plus `deceasedBoolean` and `deceasedDateTime` (each carrying `choice_of: "deceased"`). Offering the bare declarer as a field produces a document the engine rejects as an unknown element. Offer the declarer as a **type picker**, and suppress the arms as individual fields.
3. **Subtract spent cardinality.** Not in the IR, and not tracked by the engine — the editor must diff the document against the schema itself. Required-ness lives on the **parent** (`required: Vec<String>`), repeat-ness on the **child** (`array: bool`); there is no `"0..*"` string anywhere. An "add child" menu therefore needs the *parent* schema in hand, not just the element's.
4. **Suppress choice arms whose sibling is taken.** If `valueAddress` is set, do not offer `valueString`.
5. **Skip the `[x]` artifact.** Nine schemas in the R4 pack carry a literal element key `value[x]` (e.g. `observation-bmi`), a converter artifact that coexists with the proper declarer. A UI that enumerates `elements.keys()` will render a phantom field called `value[x]`. Filter keys containing `[x]`. *(This is a bug in #232's converter and should be fixed there; the editor should not have to know.)*

---

## 5. Research question 4 — extensions

Steve is right that this is where an editor lives or dies, and the answer splits cleanly in two.

### Ad-hoc extensions: **fully supported today**

The base `Extension` complex type is in the pack and gives a blind editor everything it needs: `url`, the recursive `extension`, and `value[x]` with **50 arms**. A user can attach an extension at any point that has an `extension` element — which, as the spike shows, includes `Patient.name[0]`, exactly the case Steve called out — type a URL, pick a value type, and fill it in. Nested and modifier extensions are the same walk. **No new capability required.**

### Profiled/known extensions: **blocked, and the fix is small**

The editor cannot offer *"this profile expects a `birthPlace` extension of type `Address`"*, because:

```
concrete extension definitions in the R4 pack: 0
resolve("http://hl7.org/fhir/StructureDefinition/patient-birthPlace") -> None
```

The pack generator reads three bundles:

```rust
// crates/fhir-validator/src/bin/generate_schema_packs.rs:28-29
const SOURCE_FILES: [&str; 3] =
    ["profiles-types.json", "profiles-resources.json", "profiles-others.json"];
```

`extension-definitions.json` is not among them. This is already on #232's own backlog — it is why 11 pack profiles whose `extensions` sugar references core extension URLs report `unknown-schema` today.

**This is a prerequisite, not a follow-up.** Adding the bundle to `SOURCE_FILES` is close to a one-line change, and it converts the entire "known extension" story from impossible to free: the `extensions` sugar on `FhirSchema` (`IndexMap<slice_name, Arc<FhirSchema>>`, carrying `url`/`min`/`max`) is already there, the resolver already resolves extension URLs, and tenant-uploaded extension SDs already resolve through `CompositeResolver`. The catalogue is simply empty.

Until it lands, the editor's behaviour in the gap should be explicit and honest: **offer the ad-hoc path, and say why.** "This server does not have a definition for that extension URL — you can still add it, and you pick the value type yourself." Silently degrading to a blind editor without saying so is how users end up mistrusting the tool.

---

## 6. Research question 5 — the validation feedback loop

The split in #232 is exactly the shape an editor wants, and it is worth being precise about why.

| Pass | Call | Cost | When the editor runs it |
|---|---|---|---|
| Structural | `validate_sync(&doc, &opts) -> SyncOutcome` | pure, no I/O, ~300 µs for a Patient (debug) | on every mutation, and on field blur |
| Effects | `validate(&doc, version, &opts, &handlers).await` | FHIRPath + terminology HTTP | on debounce and before save |

`SyncOutcome` returns `errors` **and** `deferred` — and, crucially, *every* error and every deferred obligation carries a dotted path (`Patient.name.0.given`) with bare numeric indices. That path maps onto an editor node by splitting on `.` and treating all-digit segments as indices. There is no ambiguity to resolve and no FHIRPath to parse.

**One sharp edge, and it changes an architectural choice.** Going through the HTTP `$validate` operation *loses information*. The REST mapping keeps only `severity`, `code`, `details.text`, `expression`:

```rust
// crates/rest/src/validation.rs:389-418
Issue::new(severity, code, error.message.clone())
    .with_expression(dotted_to_fhirpath(&error.path))
```

The structured `ErrorKind` (`unknown-element`, `slice-cardinality`, …) and the `extra` map (the failing constraint's id, the binding object) are dropped. An editor that wants to say *"this fails invariant pat-1"* and highlight the right node, rather than printing a sentence, should **call the crate in-process and use `ValidationError` directly**. `$validate` remains the right surface for external clients and for the save round-trip; it is the wrong surface for the live editing loop.

This reinforces §3: in-process, one dependency, no HTTP hop.

---

## 7. Research questions 2, 6, 7 — briefly

**Editing model (Q2).** The canonical in-flight state is **`serde_json::Value`** — because that is exactly what the validator walks, so the thing being edited and the thing being validated are the same object, with no projection to keep honest. Render two views over it: a schema-driven form, and a raw JSON view. Sync is trivial in one direction (Value → both) and is a parse in the other.

**Terminology (Q6).** Bindings are richly populated (220 of 257 schemas carry at least one). Two practical notes from the data: `Binding.value_set` **carries a version suffix** (`…/administrative-gender|4.0.1`) that must be stripped before hitting `$expand`; and the effects pass today checks **only `strength == "required"`**. For the editor: required bindings → a closed dropdown; `preferred`/`extensible` → an autocomplete that permits free text. When HTS is unavailable or the value set is huge (SNOMED subsets), degrade to a free-text code field with a visible "could not reach terminology server" note — never to a silently empty dropdown, which reads as "no valid codes exist".

**Profiles (Q7).** The user picks a profile up front (from `meta.profile`, or explicitly), and the form narrows: required slices appear, fixed values prefill and lock, `refers` scopes reference pickers. Mechanically this is just a `CompositeResolver` with the tenant/profile layer ahead of the core pack — first layer wins, and it already exists. Two honest limits to document in the UI: **slice matchers are pattern-only today** (`type`, `profile`, `binding` matchers are parsed but inert), so "add a slice instance" only works for pattern-discriminated slices; and there is **no `mustSupport` in the IR at all** (see below), so US Core's red-S emphasis cannot be rendered yet.

---

## 8. The gap that will hurt most: the IR has no words in it

This is the finding I did not expect, and it is the one most likely to derail a naive plan.

The FHIR Schema IR is **validation-only**. It carries no `short`, no `definition`, no `comment`, no `label`, no `ElementDefinition.example`. And `must_support`, `modifier` and `summary` — fields that *exist* on the struct — are **never populated by the converter**.

An editor is mostly words. Field labels, help text, the tooltip that explains what `Patient.contact.relationship` means, the emphasis that tells a US Core user which fields matter — **none of it is in this IR**, and no amount of UI work conjures it.

Three ways out, in order of preference:

1. **Extend the converter** to carry `short` / `definition` / `mustSupport` / `isModifier` / `isSummary` through from the `ElementDefinition`. This is the right fix: the data is sitting in the StructureDefinitions the generator already reads, and it belongs in the IR. Cost: a handful of fields in the converter, and pack size grows (measure it — likely tens of percent, still small).
2. Ship a **separate label pack** keyed by element path. Avoids touching #232, but creates a second artifact to keep in sync with the first. Worse.
3. **Humanise the element names** (`birthDate` → "Birth date") and render no help text. Fine for a spike, not fine for a product, and actively bad for the fields where FHIR's naming is not self-explanatory.

**Recommendation: (1), and treat it — with the extension catalogue from §5 — as the two prerequisites this editor has on #232.** Both are changes to the *generator*, both are small, and both benefit validation as much as they benefit the editor.

---

## 9. Research question 9 — prior art, and the gap we would be filling

I went looking for the tool we are about to build. **It does not exist.** That is the most useful thing this survey found, and it is worth stating precisely, because it is either an opportunity or a warning and we should know which.

| | Schema-driven form for *instances*? | Add an arbitrary extension by URL? | At an arbitrary node? | Nested / modifier extensions? |
|---|---|---|---|---|
| **Medplum** (`<ResourceForm>`) | **Yes** — the best OSS prior art | **No** — only extensions a profile declares as slices | Only where the profile slices | Yes, recursively |
| **Simplifier / Firely** | No — a **text editor** with live validation | via raw text | via raw text | via raw text |
| **Forge** | Profile editor, not an instance editor | Yes, `context`-filtered — but you are editing a *definition* | Only since 2025.1 | Yes |
| **HAPI** | **No** schema-driven editor at all | — | — | — |
| **Aidbox** | No — inline JSON/YAML editing | via raw text | via raw text | via raw text |
| **clinFHIR** (2016) | Yes — closest conceptual ancestor | Yes, from a conformance server | **Root only** | Explicitly unsupported |
| **SDC / `$questionnaire`** | Generated form | **No** — only profile-declared extensions | No | No |

Three things fall out of that table.

**The best schema in the world has no editor on it.** Health Samurai authored FHIR Schema — the IR we are building on — and their own forms product (Formbox) is built on **SDC/Questionnaire**, not on their schema. Aidbox edits arbitrary resources as **raw JSON**. Firely, the most mature commercial vendor in FHIR, ships a **tree editor for definitions and a text editor for instances**. That is a signal, not an accident: everyone who got close to this decided the general case was not worth it and shipped a text box.

Simplifier is worth pinning down precisely, because it is the tool a reasonable person would assume already does this. It does not. Firely's own documentation describes the entire instance-editing surface as:

> *"**Edit**: Update by editing the last version (opens a **XML-editor** in a small window where you can directly edit the XML code of your resource)"*
> *"**Editor**: … opens a stand-alone full screen **XML-editor** in a different tab …"*
> — [docs.fire.ly, Resources](https://docs.fire.ly/projects/Simplifier/simplifierResources.html)

And their 2024 online-editor launch, the most recent word on it, is explicitly a *code* editor:

> *"all file types, with special capabilities for editing **XML and JSON resources** (with code highlighting and live validation) and **FHIR Shorthand** files (with code highlighting, live validation and live rendering)"*
> — [Forge or FSH? Introducing the online resource editor](https://simplifier.net/organization/firely/news/176)

**Forge**, the tool people conflate with this, edits **profiles**, not instances — its `Extend` button attaches an extension to an element of a *StructureDefinition*, and it does filter the offered extensions by `context` ([docs](https://docs.simplifier.net/projects/Forge/features/DefineExtensions.html)). That context filtering is the one idea worth taking (see below); the editing target is not ours.

**We then went and used it**, rather than trusting the docs — a free account, an R4 project, and a Patient carrying two extensions: `patient-birthPlace` at the root and `humanname-own-name` hanging off `name`, the "extension on a name" case Steve flagged as the hard one. What the tool does, in its own words and behaviour:

- The **`Update` menu offers exactly three things**: `Upload`, `FHIR Read`, and — labelled by Firely themselves — **`Edit: use online text editor`**. That is the entire instance-editing surface. It opens a **JSON text editor with syntax highlighting and line numbers**. To add an extension you type the JSON block by hand.
- The resource *viewer* renders a collapsible **tree**, so Simplifier can clearly do it — it just does not let you **edit** there. And the tree shows extensions **raw**: `url: …/patient-birthPlace` and `value → city: Duckburg`. It never resolves the extension definition, never says "Birth Place", never says the value is an `Address`. **Even their viewer has no semantic help for extensions.** (Credit where due: it does collapse `valueAddress` to `value`, so it knows what a `value[x]` is.)
- Editing `"gender": "male"` into `"gender": "masculino"` — a violation of a *required* binding — **saved silently**. "Your file was saved." No validation at the point of editing.
- Validation is a **separate tool at a separate URL** (`/validator?scope=…`), which you go and run. Errors appear as a **list in a side panel**; the offending line in the code pane is not marked.
- Their instance model is telling: the resource is tagged **`type: Example of Patient`**, and the project page says *"Any resource instance … will become an **example** resource in your project."* Instances are illustrations for profiles, not first-class objects.

**Two things they do better than most, and we should take both:**

1. Their validator reports **`At Patient.gender, line 18, position 24`** — a FHIRPath *and* a source position. That is a better anchor than the prior art manages (Medplum has to *guess* the field by fuzzy-matching FHIRPath strings). It is the same anchoring our `validate_sync` hands us for free (§6), and it confirms the shape is right.
2. Their paid editor promises **live validation on every keystroke**, and their own framing is that this is what makes the blank page less intimidating. We can go further than they can: theirs is a server round-trip, ours is a pure ~300 µs function call.

**And one thing to avoid:** the error surfaced as `Exception: CodedValidationException: Value 'masculino' is not a correct code for valueset 'AdministrativeGender'`. Leaking a .NET exception class name at a clinical user is a message written for the programmer, not the person. Our `OperationOutcome` mapping should be held to a higher bar.

**Limits of this evidence, stated plainly:** we exercised the **free tier**. Simplifier advertises a paid *"full online editor"* we did not use. Firely's own launch post describes that one as *"XML and JSON resources (with code highlighting and live validation) and FHIR Shorthand files"* — a better **code** editor, not a schema-driven form. The conclusion holds, but it rests on their description, not our hands.

**The one team that did build it hit the wall at extensions.** Medplum's form is genuinely schema-driven and profile-aware, and its `getElementsToRender` **skips any `extension` element that has no slices** — their own comment says *"an extension property without slices has no nested extensions."* The consequence: **on an unprofiled resource, Medplum's form cannot add an extension at all.** Since most resources carry no `meta.profile`, that is most of the time.

**So the state of the art is a fork**, and both prongs are bad:

> *Either* you get semantic help — a type-correct `value[x]` widget, cardinality, context filtering — **and** you can only touch extensions somebody pre-declared in a profile (Medplum, SDC).
> *Or* you get total freedom — any extension, anywhere, nested — **and** you are editing raw JSON with no help whatsoever (Simplifier, Aidbox, HAPI, FRED).

**The union is unbuilt in open source.** And §2 of this document is the evidence that we can build it: the base `Extension` type gives us `url`, the 50 `value[x]` arms, and recursive `extension` — so the *freedom* prong is already ours, today, at any node, nested, for free. Add the extension catalogue from §5 and the *help* prong arrives too. **That union is the differentiating feature of this editor**, and it is the one thing in this whole design worth being ambitious about.

### What we should copy

- **Medplum's conceptual pipeline**: an internal, pre-resolved schema model (theirs is a hand-rolled FHIR Schema — same shape as ours), a per-type input dispatch, recursion into complex types, and **`applyDefaultValuesToResource`** — pre-fill `fixed`/`pattern` values from the profile so nobody types `system: http://loinc.org` by hand.
- **clinFHIR's stance, which is the correct one**: *"extensions are normal in FHIR, remember"* — render them **inline with ordinary elements, tinted differently**, not in a separate "Extensions" panel.
- **Forge's `context` filtering** when offering extensions at a node — an extension declares which elements it may attach to, and honouring that is the only thing that makes an extension picker usable rather than a list of 400 URLs. (Note this needs the extension catalogue from §5 before it can work at all.)
- **Simplifier's keystroke-level validation feedback** — and we can beat it, because theirs is a server round-trip and ours is a pure function call.
- **The three-pane layout** — tree navigator / node editor / live JSON + validate. clinFHIR and FRED converged on it independently, a decade apart.

### What we should not copy

- **Do not build on `StructureDefinition/$questionnaire` + an SDC renderer.** The operation's own spec text says the approach *"has limitations that will make it less optimal than custom-defined interfaces."* It cannot represent primitive extensions, `contained`, element `id`s, or `Narrative.div`; LHC-Forms does not implement the extraction half of the round-trip; and it destroys the validation story, because your errors end up in QuestionnaireResponse-space while `issue.expression` is a FHIRPath over the *resource*. It is a tempting shortcut with a hard ceiling.
- **Do not gate extension editing on a profile.** It is the single most-cited limitation of the best tool in the field.
- **Do not render extensions as a JSON escape hatch and call it done.** If the escape hatch is the only way to edit an extension, the schema-driven form has no reason to exist.
- **Do not fork FRED or clinFHIR.** Read them. FRED is DSTU2/STU3, unmaintained, and its own README says *"code is rough, there are bugs."*

---

## 10. The problems that break everyone else — and where we stand on each

The survey converged on a short list of things that every tool in this category gets wrong. This is the honest risk register for the prototype.

**1. Primitive extensions — the quiet data-loss bug, and the one that would bite us first.**
FHIR splits an extended primitive across two JSON keys: `birthDate` **and** `_birthDate`. Arrays are worse — `given: ["A", "B"]` with `_given: [null, {extension: [...]}]`, **positionally null-padded**. Every schema-driven form surveyed either drops the `_`-sibling on save or refuses to render it. `$questionnaire` cannot express it at all.

**Our round-trip design is immune to this by construction, and that is not an accident — it is the main reason to choose it.** Because the in-flight state *is* the `serde_json::Value` and we mutate it in place, **anything we do not render, we also do not destroy.** A form-projection architecture (fields in, fields out) loses whatever it did not model. We should still *render* `_`-siblings eventually, but on day one we will not silently eat them, and no other tool in the table can say that.

**2. Arbitrary extensions at arbitrary depth.** Covered above — this is the gap, and §2 shows the IR carries us.

**3. `modifierExtension` is a safety issue nobody surfaces.** An unrecognised modifier extension means *"do not process this resource"*. Every editor surveyed renders it identically to a plain extension. **We should not.** It needs a visible, different affordance and a warning. Note this collides with §8: the IR has a `modifier` field that the converter **never populates** — so today we cannot even detect one from the schema. We can detect the *element name* (`modifierExtension`), which is enough to start.

**4. Validating a half-finished resource.** Nobody has solved this. A resource under construction is *always* invalid, so tools either spam errors on every keystroke or dump forty issues at save. We have an unusual advantage: `validate_sync` is pure and ~300 µs, so we can afford to validate continuously — which means the problem becomes *presentation*, not cost. Proposal: **suppress `required`-kind errors on nodes the user has not yet touched**, and show them as "incomplete" rather than "wrong"; show every other error immediately. That distinction — *not yet entered* vs *wrong* — is the thing no tool makes, and it costs us a `HashSet` of touched paths.

**5. Slicing round-trip.** Taking an *existing* array, assigning each item to the slice it belongs to via the discriminator, editing one slice without corrupting the "open" items that match no slice, and re-serialising in an order that satisfies `ordered`/`openAtEnd`. Nobody does this reliably. Combined with §7's note that **only `pattern` matchers are live today**, this is the part of the prototype most likely to be descoped, and we should be upfront about that rather than discover it in week three.

**6. Which profile am I editing against?** `meta.profile` can list several, and they can conflict. Medplum takes a single `profileUrl` and has open bugs about staleness. We should make the choice explicit and visible in the UI, and treat multi-profile as an acknowledged not-yet.

### One place we are better off than the prior art

Medplum's error→field mapping is *fuzzy string matching* on FHIRPath expressions — strip array indexes from both sides, tolerate a leading `resourceType.`, then compare. It is 40 lines and it is the entire validation UX, and it is a hack forced on them because they only have the wire-format `OperationOutcome` to work with.

**We do not need the hack.** Calling the crate in-process (§6) hands us `ValidationError.path` as a structured dotted path with bare numeric indices — the exact node, no parsing, no fuzz. This is a concrete, unglamorous benefit of the in-process choice, and it is worth the dependency on its own.

---

## 11. Research question 8 — the technology call

The honest framing first. **`crates/ui` has no form primitives at all.** The most interaction-heavy thing built to date is the visual search builder, and it is **1,008 lines of hand-written vanilla JavaScript** (`assets/saved-queries.js`). A schema-driven resource editor is comfortably an order of magnitude more interaction. Pretending otherwise would be how we end up with 6,000 lines of untested JS and no build step to tame it.

But there is a property of this problem that changes the calculus, and it comes from the measurements above:

**FHIR resources are small, and the schema lookup is microseconds.** A Patient is a few KB. `validate_sync` is ~300 µs. Resolution is a hashmap hit.

That makes an unfashionable design the right one:

> **The document is the state, and it round-trips on every structural mutation.**
>
> "Add a node", "remove a node", "pick a `value[x]` type" each POST the whole in-flight document plus the mutation. The server applies it, re-runs `validate_sync`, re-renders the affected subtree, and htmx swaps it back — errors already anchored to their nodes, because the validator hands back the paths.

No client-side document model. No session state on the server. No SPA, no build step, no CDN — the crate's stance survives intact. And the editor's hardest logic (schema projection, cardinality accounting, validation) lives in Rust, where it is testable, rather than in JavaScript, where it would be a second implementation of the same rules.

What stays on the client is only what must: text input (native form fields — you do not round-trip a keystroke), a typeahead filter over the "add node" list, and keyboard navigation. That is a **scoped island of vanilla JS, ~200-300 lines**, in the same idiom as `theme.js` and `nav.js`. No framework.

**Where this design would break, stated plainly:** a very large resource (a Bundle with hundreds of entries, a big Questionnaire) makes whole-document round-trips wasteful, and deep re-render will feel laggy. The prototype should **measure that ceiling explicitly** and we should decide, with a number in hand, whether a client-side island for the document model is warranted for those cases. I would rather find the ceiling than assume it.

**Rejected: a client-side SPA island (React/Vue) with the schema shipped to the browser.** Tempting — the whole R4 pack is only 92 KB gzipped, so it *would* fit. But it buys interactivity at the cost of a build step, a CDN-or-vendoring problem, a duplicate implementation of cardinality/slicing rules in JS, and a second place for validation to disagree with the server. The round-trip design gets most of the responsiveness for none of that.

---

## 12. Prototype scope and staging

The prototype must demonstrate, end to end, on R4:

1. Load a resource from HFS by type + id; start a new empty one.
2. Render a schema-driven, spec-ordered form/tree.
3. **Add a node at any valid point**, offered from the IR — including a `value[x]` type pick and a repeating "add another".
4. Remove a node.
5. **Add/edit/remove extensions** — one profiled, one ad-hoc URL, one nested.
6. Inline validation anchored to the offending node.
7. Save back (PUT/POST), surfacing the `OperationOutcome` on rejection.

Staged, with the dependencies honest:

| Stage | What | Depends on |
|---|---|---|
| **0** | **Prerequisites in #232**: `extension-definitions.json` into the pack generator; carry `short`/`definition`/`mustSupport` through the converter; drop the `[x]` key artifact. | — |
| **1** | `addable()` + node-path projection, in the schema crate, with tests. The load-bearing piece; everything else is presentation. | 0 |
| **2** | Read-only schema-driven render of an existing resource, spec-ordered. htmx, no JS. | 1 |
| **3** | Structural mutation: add / remove / choice-pick, whole-document round-trip. | 2 |
| **4** | Extensions: ad-hoc first (works today), profiled once stage 0 lands. | 3 |
| **5** | Validation loop: `validate_sync` per mutation, async effects on debounce, errors anchored by path. | 3 |
| **6** | Terminology-backed pickers for bound fields; save + `OperationOutcome`. | 5 |
| **7** | Measure: largest resource that round-trips comfortably. Decide on a client document model *with the number in hand*. | 3 |

Explicitly out of scope, as the issue says: profile authoring, Questionnaire/SDC rendering, bulk editing, offline conflict resolution.

---

## 13. What we are asking the reviewers to decide

1. **The #215 question, restated for editors.** If validation goes the codegen route, does the FHIR Schema IR survive as **runtime-queryable data**? A compiled validator cannot drive an editor (§1). The editor — and #237 and #238 behind it — need a yes.

2. **The two prerequisites on #232** (§5, §8): the **extension catalogue** (`extension-definitions.json` is simply not in the pack generator's source list) and **human-facing metadata** in the IR (`short`/`definition`/`mustSupport` are never carried through, so the editor has no field labels and no help text). Both are generator changes, both are small, and both improve validation as much as they improve the editor. Neither is optional.

3. **The technology call** (§11): whole-document round-trip over htmx with a small vanilla-JS island — accepting that we find the size ceiling by measuring it, not by guessing.

4. **How ambitious to be about extensions.** The survey (§9) says the thing we would be building — *any extension, at any node, nested, with the schema's help* — **is unbuilt in open source**, and §2 says our IR already carries most of it. That is either the feature that makes this editor worth having, or it is scope creep. I think it is the former and I would build toward it deliberately. Worth a decision rather than a drift.

---

## Appendix: what was measured, not assumed

Everything numeric in this document came from running against the real R4 pack on `feat/fhir-validator`, not from reading docs:

- 257 schemas (192 resources / 44 complex types / 20 primitives / 1 logical); pack is 92 KB gzipped, **1.21 MB resident**.
- `Patient` is **4.8 KB**, 29 first-level elements, already flat (inherited members present without walking `base`).
- **0** concrete extension definitions; **0** populated `mustSupport` / `modifier` / `summary`; **no** `short` / `definition` / `comment` anywhere in the IR.
- `Extension` carries **50 `value[x]` arms** and a recursive `extension`.
- 9 schemas carry a bogus literal `value[x]` element key (converter artifact).
- The spike (`docs/spikes/resource-editor-addable.js`) computes addable nodes at a cursor: **22** at a Patient root, **8** inside a `HumanName` (including `extension`), and correctly excludes spent cardinality.
