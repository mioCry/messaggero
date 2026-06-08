use async_trait::async_trait;
use messaggero::prelude::*;

use crate::ollama::OllamaClient;

const SYSTEM: &str = "\
You are a professional summarizer. \
Take the given text and produce a bullet-point summary with the key points. \
Use at most 5 bullet points. Be concise. \
Respond in the same language as the input.";

pub struct SummarizerAgent {
    ollama: OllamaClient,
}

impl SummarizerAgent {
    pub fn new(ollama_url: &str, model: &str) -> Self {
        Self {
            ollama: OllamaClient::new(ollama_url, model),
        }
    }
}

#[async_trait]
impl Agent for SummarizerAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("summarizer")
            .description("Summarizes text into bullet points")
            .url("http://localhost:3003")
            .skill("summarize", "Summarize", "Creates bullet-point summaries")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let text = req
            .message
            .text_content()
            .ok_or_else(|| AgentError::InvalidRequest("empty message".into()))?
            .to_string();

        println!("\n\x1b[1;32m▶ [Summarizer]\x1b[0m ");

        let prompt = format!("Summarize this text in bullet points:\n\n{text}");

        let (summary, timing) = self
            .ollama
            .generate_streaming(Some(SYSTEM), prompt)
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

        let mut resp = TaskResponse::completed(&req.id, Message::agent(summary));
        resp.metadata = Some(meta);
        Ok(resp)
    }
}
