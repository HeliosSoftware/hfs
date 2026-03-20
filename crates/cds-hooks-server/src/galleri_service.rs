use async_trait::async_trait;
use helios_cds_hooks::{
    Action, ActionType, Card, CdsHooksError, CdsHooksService, CdsRequest, CdsResponse, CdsService,
    Coding, Indicator, Link, LinkType, OrderSelectContext, Source, Suggestion,
};
use serde_json::{Value, json};
use std::collections::HashMap;

/// CDS Hooks service that checks order-select requests for the Galleri
/// multi-cancer early detection test and returns a coverage denial card
/// when the test is identified by CPT 81479 or the Grail-specific code.
pub struct GalleriCoverageService;

impl GalleriCoverageService {
    /// Check whether a draft order Bundle contains a ServiceRequest for the
    /// Galleri test. Returns the ServiceRequest id if found.
    fn find_galleri_order(draft_orders: &Value) -> Option<String> {
        let entries = draft_orders.get("entry")?.as_array()?;
        for entry in entries {
            let resource = entry.get("resource").unwrap_or(entry);
            if resource.get("resourceType").and_then(|v| v.as_str()) != Some("ServiceRequest") {
                continue;
            }
            if Self::is_galleri_service_request(resource) {
                let id = resource
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Some(id.to_string());
            }
        }
        None
    }

    /// Check if a single ServiceRequest resource is for the Galleri test.
    fn is_galleri_service_request(resource: &Value) -> bool {
        let codings = resource
            .get("code")
            .and_then(|c| c.get("coding"))
            .and_then(|c| c.as_array());

        let Some(codings) = codings else {
            return false;
        };

        for coding in codings {
            let system = coding.get("system").and_then(|v| v.as_str()).unwrap_or("");
            let code = coding.get("code").and_then(|v| v.as_str()).unwrap_or("");

            // Match on CPT 81479 (unlisted molecular pathology) or Grail's own code
            if (system == "http://www.ama-assn.org/go/cpt" && code == "81479")
                || (system == "https://www.grail.com/tests" && code == "GALLERI")
            {
                return true;
            }
        }
        false
    }

    fn build_denial_card(service_request_id: &str) -> Card {
        let mut extension = HashMap::new();
        extension.insert(
            "davinci-crd.coverage-information".to_string(),
            json!([
                {
                    "coverage": { "reference": "Coverage/cov-bcbsm-001" },
                    "covered": "not-covered",
                    "pa-needed": "no-auth",
                    "doc-needed": "no-doc",
                    "coverage-assertion-id": "ca-bcbsm-20260313-81479-GALLERI-pat456",
                    "date": "2026-03-13",
                    "coverage-detail": [
                        {
                            "code": {
                                "system": "http://hl7.org/fhir/us/davinci-crd/CodeSystem/temp",
                                "code": "ei-status",
                                "display": "Experimental/Investigational Status"
                            },
                            "value": "Determined E/I per BCBSM Joint Medical Policy, Table 1"
                        },
                        {
                            "code": {
                                "system": "http://hl7.org/fhir/us/davinci-crd/CodeSystem/temp",
                                "code": "policy-effective-date",
                                "display": "Policy Effective Date"
                            },
                            "value": "2026-03-01"
                        },
                        {
                            "code": {
                                "system": "http://hl7.org/fhir/us/davinci-crd/CodeSystem/temp",
                                "code": "policy-title",
                                "display": "Governing Medical Policy"
                            },
                            "value": "Laboratory Tests-Genetic, Molecular, and Other-Experimental/Investigational Status"
                        },
                        {
                            "code": {
                                "system": "http://hl7.org/fhir/us/davinci-crd/CodeSystem/temp",
                                "code": "ei-rationale",
                                "display": "E/I Rationale Framework"
                            },
                            "value": "ACCE Model: analytic validity, clinical validity, clinical utility not demonstrated"
                        }
                    ]
                }
            ]),
        );

        let resource_ref = format!("ServiceRequest/{}", service_request_id);

        Card {
            uuid: Some(uuid::Uuid::new_v4().to_string()),
            summary: "Galleri (CPT 81479) — Not Covered: Experimental/Investigational".to_string(),
            detail: Some(
                "The Galleri multicancer early detection test (CPT 81479, 81599, 86849) has been \
                 determined to be **Experimental/Investigational** under BCBSM Joint Medical \
                 Policy effective 3/1/26 (Title: *Laboratory Tests — Genetic, Molecular, and \
                 Other — Experimental/Investigational Status*, Table 1). \n\n\
                 BCBSM policy basis: The test has not demonstrated definitive analytic validity, \
                 clinical validity, and clinical utility per the ACCE evaluation framework, nor \
                 a definitive and consistent impact on clinical outcomes.\n\n\
                 This test is **not a covered benefit** for contracts that exclude reimbursement \
                 for investigational services. Reimbursement will not be provided regardless of \
                 medical necessity documentation."
                    .to_string(),
            ),
            indicator: Indicator::Critical,
            source: Source {
                label: "BCBSM/BCN Coverage Rules — CRD Service".to_string(),
                url: Some("https://payer.bcbsm.example.org/crd".to_string()),
                icon: None,
                topic: Some(Coding {
                    system: "http://hl7.org/fhir/us/davinci-crd/CodeSystem/temp".to_string(),
                    code: "coverage-rules".to_string(),
                    display: Some("Coverage Rules".to_string()),
                }),
            },
            suggestions: Some(vec![Suggestion {
                label: "Remove Galleri order from draft".to_string(),
                uuid: Some(uuid::Uuid::new_v4().to_string()),
                is_recommended: Some(true),
                actions: Some(vec![Action {
                    action_type: ActionType::Delete,
                    description: Some(
                        "Delete the draft Galleri ServiceRequest — not a covered benefit"
                            .to_string(),
                    ),
                    resource: None,
                    resource_id: Some(resource_ref),
                }]),
                action_selection_behavior: None,
            }]),
            selection_behavior: None,
            override_reasons: None,
            links: Some(vec![Link {
                label: "BCBSM Medical Policy: Laboratory Tests — Genetic, Molecular, and Other — E/I Status (eff. 3/1/26)".to_string(),
                url: "https://www.bcbsm.com/content/dam/public/provider/documents/publish/medpollist/mp-laboratory-tests-genetic-molecular-investigational-status.pdf".to_string(),
                link_type: LinkType::Absolute,
                app_context: None,
                autolaunchable: None,
            }]),
            extension: Some(extension),
        }
    }
}

#[async_trait]
impl CdsHooksService for GalleriCoverageService {
    type Context = OrderSelectContext;

    fn definition(&self) -> CdsService {
        let mut prefetch = HashMap::new();
        prefetch.insert(
            "patient".to_string(),
            "Patient/{{context.patientId}}".to_string(),
        );
        prefetch.insert(
            "coverage".to_string(),
            "Coverage?patient={{context.patientId}}&status=active".to_string(),
        );

        CdsService {
            hook: "order-select".to_string(),
            title: Some("BCBSM Coverage Requirements Discovery — Galleri MCED Test".to_string()),
            description: "Evaluates draft ServiceRequest orders for the Galleri multi-cancer \
                          early detection test (CPT 81479) and returns coverage determination \
                          per BCBSM medical policy."
                .to_string(),
            id: "bcbsm-galleri-crd".to_string(),
            prefetch: Some(prefetch),
            usage_requirements: Some(
                "Requires order-select hook with draft ServiceRequest orders in context."
                    .to_string(),
            ),
            version: Some("1.0".to_string()),
            extension: None,
        }
    }

    async fn call(
        &self,
        _request: &CdsRequest,
        context: &OrderSelectContext,
    ) -> Result<CdsResponse, CdsHooksError> {
        tracing::info!(
            patient_id = %context.patient_id,
            selections = ?context.selections,
            "Processing order-select hook"
        );

        // Check if draft_orders is a Bundle containing the ServiceRequest directly,
        // or if the ServiceRequest was passed as a standalone resource in prefetch/context
        let galleri_id = Self::find_galleri_order(&context.draft_orders).or_else(|| {
            // Also check selections — the ServiceRequest might be directly in context
            // For prototype: check if the draftOrders itself is a ServiceRequest
            if context
                .draft_orders
                .get("resourceType")
                .and_then(|v| v.as_str())
                == Some("ServiceRequest")
                && Self::is_galleri_service_request(&context.draft_orders)
            {
                context
                    .draft_orders
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        });

        match galleri_id {
            Some(id) => {
                tracing::info!(service_request_id = %id, "Galleri test detected — returning denial card");
                let card = Self::build_denial_card(&id);
                Ok(CdsResponse::with_cards(vec![card]))
            }
            None => {
                tracing::debug!("No Galleri test found in draft orders");
                Ok(CdsResponse::empty())
            }
        }
    }

    async fn on_feedback(
        &self,
        feedback: &helios_cds_hooks::FeedbackRequest,
    ) -> Result<(), CdsHooksError> {
        for fb in &feedback.feedback {
            tracing::info!(
                card = %fb.card,
                outcome = ?fb.outcome,
                "Received feedback for Galleri CRD card"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn galleri_service_request() -> Value {
        json!({
            "resourceType": "ServiceRequest",
            "id": "sr-galleri-001",
            "status": "draft",
            "intent": "proposal",
            "subject": { "reference": "Patient/pat-456" },
            "requester": { "reference": "Practitioner/pract-123" },
            "code": {
                "coding": [
                    {
                        "system": "http://www.ama-assn.org/go/cpt",
                        "code": "81479",
                        "display": "Unlisted molecular pathology procedure"
                    },
                    {
                        "system": "https://www.grail.com/tests",
                        "code": "GALLERI",
                        "display": "Galleri Multi-Cancer Early Detection Test"
                    }
                ],
                "text": "Galleri (Multicancer Early Detection Test)"
            },
            "occurrenceDateTime": "2026-03-13"
        })
    }

    fn make_bundle(resources: Vec<Value>) -> Value {
        json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": resources.into_iter().map(|r| json!({ "resource": r })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn detects_galleri_in_bundle() {
        let bundle = make_bundle(vec![galleri_service_request()]);
        let id = GalleriCoverageService::find_galleri_order(&bundle);
        assert_eq!(id, Some("sr-galleri-001".to_string()));
    }

    #[test]
    fn detects_galleri_standalone() {
        let sr = galleri_service_request();
        assert!(GalleriCoverageService::is_galleri_service_request(&sr));
    }

    #[test]
    fn ignores_non_galleri() {
        let sr = json!({
            "resourceType": "ServiceRequest",
            "id": "sr-cbc-001",
            "code": {
                "coding": [{
                    "system": "http://www.ama-assn.org/go/cpt",
                    "code": "85025",
                    "display": "Complete blood count"
                }]
            }
        });
        assert!(!GalleriCoverageService::is_galleri_service_request(&sr));
    }

    #[test]
    fn denial_card_has_critical_indicator() {
        let card = GalleriCoverageService::build_denial_card("sr-galleri-001");
        assert_eq!(card.indicator, Indicator::Critical);
        assert!(card.summary.contains("Not Covered"));
        assert!(card.extension.is_some());
    }

    #[tokio::test]
    async fn service_returns_card_for_galleri() {
        let service = GalleriCoverageService;
        let bundle = make_bundle(vec![galleri_service_request()]);

        let request = CdsRequest {
            hook: "order-select".to_string(),
            hook_instance: uuid::Uuid::new_v4().to_string(),
            fhir_server: None,
            fhir_authorization: None,
            context: json!({
                "userId": "Practitioner/pract-123",
                "patientId": "pat-456",
                "selections": ["ServiceRequest/sr-galleri-001"],
                "draftOrders": bundle
            }),
            prefetch: None,
            extension: None,
        };

        let context: OrderSelectContext = serde_json::from_value(request.context.clone()).unwrap();

        let response = service.call(&request, &context).await.unwrap();
        assert_eq!(response.cards.len(), 1);
        assert_eq!(response.cards[0].indicator, Indicator::Critical);
    }

    #[tokio::test]
    async fn service_returns_empty_for_non_galleri() {
        let service = GalleriCoverageService;
        let bundle = make_bundle(vec![json!({
            "resourceType": "ServiceRequest",
            "id": "sr-cbc-001",
            "code": {
                "coding": [{
                    "system": "http://www.ama-assn.org/go/cpt",
                    "code": "85025",
                    "display": "Complete blood count"
                }]
            }
        })]);

        let request = CdsRequest {
            hook: "order-select".to_string(),
            hook_instance: uuid::Uuid::new_v4().to_string(),
            fhir_server: None,
            fhir_authorization: None,
            context: json!({
                "userId": "Practitioner/pract-123",
                "patientId": "pat-456",
                "selections": ["ServiceRequest/sr-cbc-001"],
                "draftOrders": bundle
            }),
            prefetch: None,
            extension: None,
        };

        let context: OrderSelectContext = serde_json::from_value(request.context.clone()).unwrap();

        let response = service.call(&request, &context).await.unwrap();
        assert!(response.cards.is_empty());
    }

    #[test]
    fn service_definition_is_order_select() {
        let service = GalleriCoverageService;
        let def = service.definition();
        assert_eq!(def.hook, "order-select");
        assert_eq!(def.id, "bcbsm-galleri-crd");
        assert!(def.prefetch.is_some());
    }
}
