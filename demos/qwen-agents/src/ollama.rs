use std::io::Write;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

pub struct OllamaClient {
    base_url: String,
    model: String,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    /// Disables Qwen3 chain-of-thought reasoning (supported in Ollama >= 0.6.4).
    think: bool,
}

#[derive(Deserialize)]
struct StreamChunk {
    response: String,
    #[serde(default)]
    done: bool,
    /// Qwen3 thinking tokens arrive in this separate field when think=true.
    /// When think=false it is always empty; we ignore it regardless.
    #[serde(default)]
    thinking: String,
}

/// Timing breakdown returned alongside generated text.
pub struct GenerateTiming {
    /// Time spent waiting for the first token (network + model load).
    pub time_to_first_token: Duration,
    /// Total time the LLM spent generating (including first token wait).
    pub llm_total: Duration,
}

/// Prefix injected into every system prompt to disable Qwen3 reasoning via
/// the soft-token mechanism, as a fallback for older Ollama versions that
/// do not honour the `think: false` API field.
const NO_THINK_PREFIX: &str = "/no_think\n\n";

impl OllamaClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Stream the LLM response token-by-token to stdout, returning the
    /// complete text and detailed timing information.
    ///
    /// Reasoning (chain-of-thought) is disabled via two complementary mechanisms:
    /// - `think: false` in the Ollama API request body (Ollama >= 0.6.4)
    /// - `/no_think` prefix in the system prompt (all versions)
    ///
    /// Any residual `<think>...</think>` blocks that might appear in the
    /// response stream are stripped before printing and before returning.
    pub async fn generate_streaming(
        &self,
        system: Option<&str>,
        prompt: impl Into<String>,
    ) -> Result<(String, GenerateTiming), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/api/generate", self.base_url);

        // Prepend /no_think to the system prompt as a belt-and-suspenders
        // fallback for Ollama versions that do not support think: false.
        let system_with_no_think = system.map(|s| format!("{NO_THINK_PREFIX}{s}"));

        let body = GenerateRequest {
            model: self.model.clone(),
            prompt: prompt.into(),
            stream: true,
            system: system_with_no_think,
            think: false,
        };

        let start = Instant::now();
        let resp = self.http.post(&url).json(&body).send().await?;

        let mut byte_stream = resp.bytes_stream();
        let mut full_text = String::new();
        let mut line_buf = String::new();
        let mut first_token_time: Option<Duration> = None;

        // State machine for stripping <think>...</think> blocks that may
        // appear if the model ignores the no-think instructions.
        let mut inside_think_block = false;

        while let Some(chunk) = byte_stream.next().await {
            let bytes: bytes::Bytes = chunk?;
            line_buf.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(nl) = line_buf.find('\n') {
                let line = line_buf[..nl].trim().to_string();
                line_buf = line_buf[nl + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Ok(sc) = serde_json::from_str::<StreamChunk>(&line) {
                    // Ignore tokens that arrive in the dedicated thinking field.
                    if !sc.thinking.is_empty() {
                        continue;
                    }

                    let token = &sc.response;

                    // Strip any <think> / </think> markers from the response
                    // field itself (some Ollama versions mix them in).
                    if token.contains("<think>") {
                        inside_think_block = true;
                    }
                    if token.contains("</think>") {
                        inside_think_block = false;
                        if sc.done {
                            break;
                        }
                        continue;
                    }

                    if inside_think_block {
                        if sc.done {
                            break;
                        }
                        continue;
                    }

                    if first_token_time.is_none() && !token.is_empty() {
                        first_token_time = Some(start.elapsed());
                    }

                    print!("{token}");
                    std::io::stdout().flush().ok();
                    full_text.push_str(token);

                    if sc.done {
                        break;
                    }
                }
            }
        }

        if !full_text.ends_with('\n') {
            println!();
        }

        let llm_total = start.elapsed();
        let timing = GenerateTiming {
            time_to_first_token: first_token_time.unwrap_or(llm_total),
            llm_total,
        };

        Ok((full_text.trim().to_string(), timing))
    }
}
