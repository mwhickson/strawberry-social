use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatMessagePayload {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessagePayload>,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: usize,
    pub stream: bool,
}

pub struct LlmClient {
    client: Client,
    base_url: String,
    api_key: String,
}

impl LlmClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            api_key,
        }
    }

    pub async fn send_chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        
        let mut req_builder = self.client.post(&url).json(request);
        if !self.api_key.is_empty() && self.api_key != "not-needed" {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", self.api_key));
        }

        req_builder.send().await
    }
}