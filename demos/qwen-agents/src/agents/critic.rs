use async_trait::async_trait;
use messaggero::prelude::*;

use crate::ollama::OllamaClient;

const SYSTEM: &str = "\
You are a constructive critic and editor. \
You receive a piece of text and your job is to:
1. Identify any inaccuracies or gaps
2. Improve clarity and conciseness
3. Provide the improved version of the text

Format your response as:
FEEDBACK: <your feedback in 1-2 sentences>
IMPROVED: <the improved text>

Be direct and helpful. Respond in the same language as the input.";

pub struct CriticAgent {
    ollama: OllamaClient,
}

impl CriticAgent {
    pub fn new(ollama_url: &str, model: &str) -> Self {
        Self {
            ollama: OllamaClient::new(ollama_url, model),
        }
    }
}

#[async_trait]
impl Agent for CriticAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("critic")
            .description("Reviews and improves text responses using LLM")
            .url("http://localhost:3002")
            .skill("critique", "Critique", "Reviews and improves text quality")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let text = req
            .message
            .text_content()
            .ok_or_else(|| AgentError::InvalidRequest("empty message".into()))?
            .to_string();

        println!("\n\x1b[1;33m▶ [Critic]\x1b[0m ");

        let prompt = format!("Please review and improve this text:\n\n{text}");

        let (critique, timing) = self
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

        let mut resp = TaskResponse::completed(&req.id, Message::agent(critique));
        resp.metadata = Some(meta);
        Ok(resp)
    }
}
