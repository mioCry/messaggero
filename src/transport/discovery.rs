use std::collections::HashMap;
use std::sync::Arc;

use crate::core::AgentCard;
use tokio::sync::RwLock;

use super::router::{AgentEndpoint, Router};

/// Local agent registry for discovery.
///
/// Agents register their cards; other agents query the registry to discover
/// available peers and auto-populate the router.
pub struct Discovery {
    cards: Arc<RwLock<HashMap<String, (AgentCard, AgentEndpoint)>>>,
}

impl Discovery {
    pub fn new() -> Self {
        Self {
            cards: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register an agent with its card and endpoint.
    pub async fn register(&self, card: AgentCard, endpoint: AgentEndpoint) {
        let name = card.name.clone();
        self.cards.write().await.insert(name, (card, endpoint));
    }

    /// Remove an agent from the registry.
    pub async fn unregister(&self, name: &str) {
        self.cards.write().await.remove(name);
    }

    /// Find an agent card by name.
    pub async fn find(&self, name: &str) -> Option<AgentCard> {
        self.cards
            .read()
            .await
            .get(name)
            .map(|(card, _)| card.clone())
    }

    /// List all registered agent cards.
    pub async fn list(&self) -> Vec<AgentCard> {
        self.cards
            .read()
            .await
            .values()
            .map(|(card, _)| card.clone())
            .collect()
    }

    /// Find agents that have a specific skill tag.
    pub async fn find_by_tag(&self, tag: &str) -> Vec<AgentCard> {
        self.cards
            .read()
            .await
            .values()
            .filter(|(card, _)| card.skills.iter().any(|s| s.tags.iter().any(|t| t == tag)))
            .map(|(card, _)| card.clone())
            .collect()
    }

    /// Populate a router with all registered agent endpoints.
    pub async fn populate_router(&self, router: &Router) {
        let cards = self.cards.read().await;
        for (name, (_, endpoint)) in cards.iter() {
            router.register(name.clone(), endpoint.clone()).await;
        }
    }

    /// Fetch a remote agent card from an HTTP endpoint and register it.
    #[cfg(feature = "a2a")]
    pub async fn discover_remote(
        &self,
        base_url: impl Into<String>,
    ) -> Result<AgentCard, crate::core::TransportError> {
        let url = base_url.into();
        let client = super::a2a::A2AClient::new(&url);
        let card = client.agent_card().await?;
        self.register(card.clone(), AgentEndpoint::Http(url)).await;
        Ok(card)
    }
}

impl Default for Discovery {
    fn default() -> Self {
        Self::new()
    }
}
