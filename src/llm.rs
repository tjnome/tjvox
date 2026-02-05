use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

use crate::config::LlmConfig;
use crate::error::TjvoxError;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

pub struct LlmProcessor {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    prompt: String,
}

impl LlmProcessor {
    pub fn new(config: &LlmConfig) -> Result<Self, TjvoxError> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms));

        if !config.api_key.is_empty() {
            let mut headers = reqwest::header::HeaderMap::new();
            let value = format!("Bearer {}", config.api_key);
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&value)
                    .map_err(|e| TjvoxError::Llm(format!("invalid API key: {}", e)))?,
            );
            builder = builder.default_headers(headers);
        }

        let client = builder
            .build()
            .map_err(|e| TjvoxError::Llm(format!("failed to build HTTP client: {}", e)))?;

        Ok(Self {
            client,
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            prompt: config.prompt.clone(),
        })
    }

    pub async fn process(&self, text: &str) -> Result<String, TjvoxError> {
        debug!("Sending text to LLM for post-processing");

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: self.prompt.clone(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: text.to_string(),
                },
            ],
            temperature: 0.3,
        };

        let response = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| TjvoxError::Llm(format!("request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(TjvoxError::Llm(format!(
                "API returned status {}",
                response.status()
            )));
        }

        let body: ChatResponse = response
            .json()
            .await
            .map_err(|e| TjvoxError::Llm(format!("failed to parse response: {}", e)))?;

        let content = body
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| TjvoxError::Llm("no choices in response".to_string()))?;

        debug!("LLM corrected text: {}", content);
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LlmConfig;

    #[test]
    fn test_llm_processor_new() {
        let config = LlmConfig {
            enabled: true,
            endpoint: "http://localhost:11434/v1/chat/completions".to_string(),
            api_key: String::new(),
            model: "llama3".to_string(),
            prompt: "Fix grammar.".to_string(),
            timeout_ms: 5000,
        };
        let processor = LlmProcessor::new(&config);
        assert!(processor.is_ok());
    }

    #[test]
    fn test_llm_processor_with_api_key() {
        let config = LlmConfig {
            enabled: true,
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            api_key: "sk-test-key".to_string(),
            model: "gpt-4".to_string(),
            prompt: "Fix grammar.".to_string(),
            timeout_ms: 10000,
        };
        let processor = LlmProcessor::new(&config);
        assert!(processor.is_ok());
    }

    #[test]
    fn test_chat_request_serialization() {
        let request = ChatRequest {
            model: "llama3".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: "Fix grammar.".to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: "hello world".to_string(),
                },
            ],
            temperature: 0.3,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"llama3\""));
        assert!(json.contains("\"temperature\":0.3"));
    }

    #[test]
    fn test_chat_response_deserialization() {
        let json = r#"{
            "choices": [{
                "message": {
                    "content": "Hello, world!"
                }
            }]
        }"#;
        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.choices[0].message.content, "Hello, world!");
    }

    #[test]
    fn test_chat_response_empty_choices() {
        let json = r#"{"choices": []}"#;
        let response: ChatResponse = serde_json::from_str(json).unwrap();
        assert!(response.choices.is_empty());
    }
}
