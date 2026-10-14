//! Leading type identifiers (#640): the three-way rule at each evaluator site.
//!
//! | Leading identifier                        | Result                       |
//! |-------------------------------------------|------------------------------|
//! | the context type (`Patient` on a Patient) | resolves                     |
//! | a known type that is not the context type | silent empty (all policies)  |
//! | an abstract base (`Resource`)             | resolves                     |
//! | not a type, not an element (`Bogus`)      | diagnosed per policy         |

use chumsky::Parser;
use helios_fhir::{FhirResource, r4};
use helios_fhirpath::evaluator::{EvaluationContext, UnresolvedIdentifierPolicy, evaluate};
use helios_fhirpath::parser::parser;
use helios_fhirpath::{EvaluationError, EvaluationResult};
use serde_json::{Value, json};
use std::sync::Arc;

use UnresolvedIdentifierPolicy::{Error, Ignore, Warn};

const SOURCE: &str = "http://example.org/src";

fn typed(value: Value) -> FhirResource {
    let resource: r4::Resource = serde_json::from_value(value).expect("valid R4 resource");
    FhirResource::R4(Box::new(resource))
}

fn patient() -> Value {
    json!({"resourceType": "Patient", "id": "p1", "meta": {"source": SOURCE}})
}

fn observation() -> Value {
    json!({"resourceType": "Observation", "status": "final",
           "code": {"text": "BP"}, "meta": {"source": SOURCE}})
}

/// Which evaluator call site the expression is routed through.
#[derive(Clone, Copy, Debug)]
enum Site {
    /// A: resource in `resources`, no `this`.
    A,
    /// B: `this` is set.
    B,
}

fn context(site: Site, resource: Value, policy: UnresolvedIdentifierPolicy) -> EvaluationContext {
    let mut ctx = EvaluationContext::new(vec![typed(resource)]);
    if matches!(site, Site::A) {
        ctx.this = None;
    }
    ctx.set_unresolved_identifier_policy(policy);
    ctx
}

fn eval(expr: &str, ctx: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
    let parsed = parser()
        .parse(expr)
        .into_result()
        .unwrap_or_else(|e| panic!("parse error for '{expr}': {e:?}"));
    evaluate(&parsed, ctx, None)
}

fn source() -> EvaluationResult {
    EvaluationResult::fhir_string(SOURCE.to_string(), "uri")
}

fn assert_source(result: EvaluationResult) {
    // Compare on the string payload: the FHIR type tag is not what is under test.
    match result {
        EvaluationResult::String(s, _, _) => assert_eq!(s, SOURCE),
        other => panic!(
            "expected the source string, got {other:?} (reference: {:?})",
            source()
        ),
    }
}

const SITES: [Site; 2] = [Site::A, Site::B];
const ALL_POLICIES: [UnresolvedIdentifierPolicy; 3] = [Ignore, Warn, Error];

#[test]
fn row1_context_type_resolves() {
    for site in SITES {
        for policy in ALL_POLICIES {
            let ctx = context(site, patient(), policy);
            assert_source(eval("Patient.meta.source", &ctx).unwrap());
        }
    }
}

#[test]
fn row2_known_other_type_stays_silent_under_every_policy() {
    for site in SITES {
        for policy in ALL_POLICIES {
            let ctx = context(site, observation(), policy);
            let result = eval("Patient.name", &ctx)
                .unwrap_or_else(|e| panic!("{site:?}/{policy:?} must stay silent: {e:?}"));
            assert_eq!(result, EvaluationResult::Empty, "{site:?}/{policy:?}");
        }
    }
}

#[test]
fn row3_abstract_bases_resolve() {
    for site in SITES {
        for policy in ALL_POLICIES {
            let ctx = context(site, patient(), policy);
            assert_source(eval("Resource.meta.source", &ctx).unwrap());
            assert_source(eval("DomainResource.meta.source", &ctx).unwrap());
        }
    }
}

#[test]
fn row3_domain_resource_does_not_subsume_bundle() {
    let bundle = json!({"resourceType": "Bundle", "type": "collection",
                        "meta": {"source": SOURCE}});
    for site in SITES {
        for policy in ALL_POLICIES {
            let ctx = context(site, bundle.clone(), policy);
            assert_eq!(
                eval("DomainResource.text", &ctx).unwrap(),
                EvaluationResult::Empty,
                "{site:?}/{policy:?}"
            );
            // `Resource` still subsumes it.
            assert_source(eval("Resource.meta.source", &ctx).unwrap());
        }
    }
}

#[test]
fn row4_unknown_identifier_is_diagnosed_per_policy() {
    for site in SITES {
        let ctx = context(site, patient(), Error);
        match eval("Bogus.meta.source", &ctx) {
            Err(EvaluationError::SemanticError(msg)) => {
                assert!(msg.contains("Bogus"), "{msg}");
                assert!(msg.contains("Patient"), "{msg}");
            }
            other => panic!("{site:?}: expected SemanticError, got {other:?}"),
        }
        for policy in [Ignore, Warn] {
            let ctx = context(site, patient(), policy);
            assert_eq!(
                eval("Bogus.meta.source", &ctx).unwrap(),
                EvaluationResult::Empty,
                "{site:?}/{policy:?}"
            );
        }
    }
}

#[test]
fn lowercase_unknown_name_is_just_an_absent_element() {
    for policy in ALL_POLICIES {
        let ctx = context(Site::B, patient(), policy);
        assert_eq!(
            eval("bogus.meta.source", &ctx).unwrap(),
            EvaluationResult::Empty
        );
    }
}

#[test]
fn site_c_define_variable_path() {
    let ctx = context(Site::B, patient(), Error);
    assert_source(eval("defineVariable('v', 1).select(Resource.meta.source)", &ctx).unwrap());
    assert!(eval("defineVariable('v', 1).select(Bogus.meta.source)", &ctx).is_err());
    let lenient = context(Site::B, patient(), Warn);
    assert_eq!(
        eval("defineVariable('v', 1).select(Bogus.meta.source)", &lenient).unwrap(),
        EvaluationResult::Empty
    );
}

fn bundle() -> Value {
    json!({"resourceType": "Bundle", "type": "collection",
           "entry": [{"resource": patient()}]})
}

/// Site D: lambda items are typed resources, where the abstract base used to
/// miss because the old guard required `type_info: None`.
#[test]
fn site_d_lambda_over_typed_items() {
    let count = |expr: &str, policy| {
        let ctx = context(Site::B, bundle(), policy);
        eval(
            &format!("Bundle.entry.resource.where({expr}).count()"),
            &ctx,
        )
    };
    for policy in ALL_POLICIES {
        assert_eq!(
            count("Resource.meta.source.exists()", policy).unwrap(),
            EvaluationResult::integer(1)
        );
        assert_eq!(
            count("DomainResource.meta.source.exists()", policy).unwrap(),
            EvaluationResult::integer(1)
        );
        // Known type that is not the item's type: silent.
        assert_eq!(
            count("Observation.code.exists()", policy).unwrap(),
            EvaluationResult::integer(0)
        );
    }
    assert!(count("Bogus.meta.source.exists()", Error).is_err());
    assert_eq!(
        count("Bogus.meta.source.exists()", Warn).unwrap(),
        EvaluationResult::integer(0)
    );
    assert_eq!(
        count("Bogus.meta.source.exists()", Ignore).unwrap(),
        EvaluationResult::integer(0)
    );
}

/// The case a naive "leading identifier must match" fix breaks.
#[test]
fn union_type_dispatch_still_returns_the_matching_member() {
    for site in SITES {
        for policy in ALL_POLICIES {
            let ctx = context(site, observation(), policy);
            let result = eval("Patient.name.family | Observation.code.text", &ctx)
                .unwrap_or_else(|e| panic!("{site:?}/{policy:?}: {e:?}"));
            match result {
                EvaluationResult::String(s, _, _) => assert_eq!(s, "BP"),
                EvaluationResult::Collection { items, .. } => {
                    assert_eq!(items.len(), 1, "{items:?}")
                }
                other => panic!("{site:?}/{policy:?}: expected \"BP\", got {other:?}"),
            }
        }
    }
}

#[test]
fn type_operators_accept_abstract_bases() {
    let ctx = context(Site::B, patient(), Error);
    for expr in [
        "$this is Resource",
        "$this is DomainResource",
        "$this.is(Resource)",
        "$this.is(DomainResource)",
    ] {
        assert_eq!(
            eval(expr, &ctx).unwrap(),
            EvaluationResult::boolean(true),
            "{expr}"
        );
    }
    let bundle_ctx = context(Site::B, bundle(), Error);
    assert_eq!(
        eval("$this is Resource", &bundle_ctx).unwrap(),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        eval("$this is DomainResource", &bundle_ctx).unwrap(),
        EvaluationResult::boolean(false)
    );
    // Truly unknown names are still rejected by `is`.
    assert!(eval("$this is Bogus", &ctx).is_err());
}

#[test]
fn of_type_accepts_abstract_bases() {
    let ctx = context(Site::B, bundle(), Error);
    assert_eq!(
        eval("Bundle.entry.resource.ofType(Resource).count()", &ctx).unwrap(),
        EvaluationResult::integer(1)
    );
    assert_eq!(
        eval("Bundle.entry.resource.ofType(DomainResource).count()", &ctx).unwrap(),
        EvaluationResult::integer(1)
    );
    assert_eq!(
        eval("Bundle.ofType(DomainResource).count()", &ctx).unwrap(),
        EvaluationResult::integer(0)
    );
}

/// Strict mode keeps reporting a wrong-type leading name as before
/// (conformance `testSimpleWithWrongContext`).
#[test]
fn strict_mode_wrong_context_still_errors() {
    for policy in ALL_POLICIES {
        let mut ctx = context(Site::B, patient(), policy);
        ctx.set_strict_mode(true);
        assert!(eval("Encounter.name.given", &ctx).is_err(), "{policy:?}");
    }
}

#[test]
fn policy_is_inherited_by_clones_and_child_contexts() {
    let ctx = context(Site::B, patient(), Error);
    assert_eq!(ctx.clone().unresolved_identifier_policy, Error);
    assert_eq!(
        ctx.create_child_context().unresolved_identifier_policy,
        Error
    );
    assert_eq!(
        EvaluationContext::new_empty_with_default_version().unresolved_identifier_policy,
        Warn
    );
}

#[test]
fn untyped_object_context_is_diagnosed_too() {
    let mut map = std::collections::HashMap::new();
    map.insert(
        "resourceType".to_string(),
        EvaluationResult::string("Patient".to_string()),
    );
    let mut ctx = EvaluationContext::new_empty_with_default_version();
    ctx.set_this(EvaluationResult::object(map));
    ctx.set_unresolved_identifier_policy(Error);
    assert!(eval("Bogus.id", &ctx).is_err());
    assert_eq!(eval("Resource.id", &ctx).unwrap(), EvaluationResult::Empty);
    let _ = Arc::new(());
}
