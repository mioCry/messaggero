use async_trait::async_trait;
use messaggero::prelude::*;

use crate::ollama::OllamaClient;

const SYSTEM: &str = "\
You are a knowledgeable research assistant. \
When given a question or topic, provide a clear, factual, and well-structured answer. \
Be concise but thorough. Answer in the same language as the question.";

pub struct ResearcherAgent {
    ollama: OllamaClient,
}

impl ResearcherAgent {
    pub fn new(ollama_url: &str, model: &str) -> Self {
        Self {
            ollama: OllamaClient::new(ollama_url, model),
        }
    }
}

#[async_trait]
impl Agent for ResearcherAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("researcher")
            .description("Answers questions using LLM knowledge")
            .url("http://localhost:3001")
            .skill("research", "Research", "Answers factual questions")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let question = req
            .message
            .text_content()
            .ok_or_else(|| AgentError::InvalidRequest("empty message".into()))?
            .to_string();

        println!("\n\x1b[1;34m▶ [Researcher]\x1b[0m ");

        let (text, timing) = self
            .ollama
            .generate_streaming(Some(SYSTEM), question)
            .await
            .map_err(|e| AgentError::Internal(e.to_string()))?;

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "llm_us".to_string(),
            timing.llm_total.as_micros().to_string(),
        );
        meta.insert(
            "llm_ms".to_string(),
            timing.llm_total.as_millis().to_string(),
        );
        meta.insert(
            "ttft_us".to_string(),
            timing.time_to_first_token.as_micros().to_string(),
        );
        meta.insert(
            "ttft_ms".to_string(),
            timing.time_to_first_token.as_millis().to_string(),
        );

        let mut resp = TaskResponse::completed(&req.id, Message::agent(text));
        resp.metadata = Some(meta);
        Ok(resp)
    }
}
