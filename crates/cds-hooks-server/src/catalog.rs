//! The services this server offers, as one list the router can address by id.
//! Services are shared: a client sees, and may call, only the ones its
//! registry entry enables, and sees them with its own prefetch templates when
//! it overrides them.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use helios_cds_hooks::{
    CdsHooksError, CdsHooksService, CdsRequest, CdsResponse, CdsService, FeedbackRequest,
};

use crate::clients::Client;

/// A service with its context type erased, so services of different hooks
/// can share one catalog.
#[async_trait]
pub trait DynService: Send + Sync {
    fn definition(&self) -> CdsService;
    async fn invoke(&self, request: &CdsRequest) -> Result<CdsResponse, CdsHooksError>;
    async fn feedback(&self, feedback: &FeedbackRequest) -> Result<(), CdsHooksError>;
}

#[async_trait]
impl<T: CdsHooksService> DynService for T {
    fn definition(&self) -> CdsService {
        CdsHooksService::definition(self)
    }

    async fn invoke(&self, request: &CdsRequest) -> Result<CdsResponse, CdsHooksError> {
        let context = self.extract_context(request)?;
        self.call(request, &context).await
    }

    async fn feedback(&self, feedback: &FeedbackRequest) -> Result<(), CdsHooksError> {
        self.on_feedback(feedback).await
    }
}

#[derive(Default)]
pub struct ServiceCatalog {
    services: HashMap<String, Arc<dyn DynService>>,
    order: Vec<String>,
}

impl ServiceCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<T: CdsHooksService + 'static>(mut self, service: T) -> Self {
        let id = CdsHooksService::definition(&service).id;
        self.order.push(id.clone());
        self.services.insert(id, Arc::new(service));
        self
    }

    pub fn ids(&self) -> Vec<String> {
        self.order.clone()
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn DynService>> {
        self.services.get(id).cloned()
    }

    /// The service, when the client has it enabled.
    pub fn for_client(&self, client: &Client, id: &str) -> Option<Arc<dyn DynService>> {
        client.has_service(id).then(|| self.get(id)).flatten()
    }

    /// What the client sees in discovery: its enabled services, in catalog
    /// order, each with the client's prefetch templates when it set any.
    pub fn definitions_for(&self, client: &Client) -> Vec<CdsService> {
        self.order
            .iter()
            .filter(|id| client.has_service(id))
            .filter_map(|id| self.services.get(id))
            .map(|service| {
                let mut definition = service.definition();
                if let Some(templates) = client.prefetch.get(&definition.id) {
                    definition.prefetch = Some(templates.clone());
                }
                definition
            })
            .collect()
    }
}
