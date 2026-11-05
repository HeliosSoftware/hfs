//! The reference service: `patient-view` answered with one informational
//! card naming the patient in context. It is the smallest complete
//! `CdsHooksService`, the one the development registry enables, and what the
//! tests exercise. It carries no clinical logic.

use std::collections::HashMap;

use async_trait::async_trait;
use helios_cds_hooks::{
    Card, CdsHooksError, CdsHooksService, CdsRequest, CdsResponse, CdsService,
    hooks::{HookContext, PatientViewContext},
};

pub const SERVICE_ID: &str = "patient-greeter";

pub struct PatientGreeter;

#[async_trait]
impl CdsHooksService for PatientGreeter {
    type Context = PatientViewContext;

    fn definition(&self) -> CdsService {
        CdsService {
            hook: PatientViewContext::HOOK_NAME.to_string(),
            title: Some("Patient greeter".to_string()),
            description: "One informational card naming the patient whose chart was opened."
                .to_string(),
            id: SERVICE_ID.to_string(),
            prefetch: Some(HashMap::from([(
                "patient".to_string(),
                "Patient/{{context.patientId}}".to_string(),
            )])),
            usage_requirements: None,
            version: None,
            extension: None,
        }
    }

    async fn call(
        &self,
        _request: &CdsRequest,
        context: &PatientViewContext,
    ) -> Result<CdsResponse, CdsHooksError> {
        let card = Card::info(
            format!("Viewing patient {}", context.patient_id),
            "Helios CDS Hooks",
        );
        Ok(CdsResponse::with_cards(vec![card]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_cds_hooks::Indicator;
    use serde_json::json;

    #[tokio::test]
    async fn answers_patient_view_with_one_informational_card() {
        let request: CdsRequest = serde_json::from_value(json!({
            "hook": "patient-view",
            "hookInstance": "6c3a1c7e-3b1f-4b2e-9b1e-1f2d3c4b5a69",
            "context": { "userId": "Practitioner/p-1", "patientId": "pat-1" }
        }))
        .unwrap();
        let context = PatientGreeter.extract_context(&request).unwrap();
        let response = PatientGreeter.call(&request, &context).await.unwrap();
        assert_eq!(response.cards.len(), 1);
        assert_eq!(response.cards[0].indicator, Indicator::Info);
        assert!(response.cards[0].summary.contains("pat-1"));
        assert_eq!(PatientGreeter.definition().hook, "patient-view");
    }
}
