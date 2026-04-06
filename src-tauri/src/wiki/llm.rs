use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    response_format: ResponseFormat,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    r#type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifyResponse {
    pub skip: bool,
    #[serde(default)]
    pub topics: Vec<ClassifiedTopic>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifiedTopic {
    pub topic: String,
    pub topic_ko: Option<String>,
    pub category: String,
    pub relevance: f64,
}

#[derive(Debug, Deserialize)]
pub struct DedupResponse {
    pub same: bool,
    pub confidence: f64,
}

#[derive(Debug)]
pub enum LlmError {
    Http(reqwest::Error),
    Parse(String),
    Api(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(e) => write!(f, "HTTP error: {}", e),
            LlmError::Parse(e) => write!(f, "Parse error: {}", e),
            LlmError::Api(e) => write!(f, "API error: {}", e),
        }
    }
}

impl std::error::Error for LlmError {}

impl LlmClient {
    pub fn new(api_key: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            model: "gpt-4o-mini".to_string(),
        }
    }

    pub async fn validate_key(&self) -> Result<bool, LlmError> {
        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Reply with just: ok".to_string(),
            }],
            temperature: 0.0,
            response_format: ResponseFormat {
                r#type: "text".to_string(),
            },
        };

        match self.call_api(&req).await {
            Ok(_) => Ok(true),
            Err(LlmError::Api(msg)) if msg.contains("401") || msg.contains("invalid") => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub async fn classify_message(
        &self,
        chat_title: &str,
        timestamp: i64,
        text: &str,
    ) -> Result<ClassifyResponse, LlmError> {
        let system = r#"You are a crypto/finance message classifier for a Telegram archive.
Classify the message into one or more topics. Return ONLY valid JSON.

Rules:
- topics: array of 1-3 topics this message relates to
- Each topic: concise English title (e.g., "ETH Layer 2 Fees", "Solana Outage")
- topic_ko: Korean title if inferrable, else null
- category: one of [DeFi, Trading, L1/L2, NFT, Airdrop, Regulation, Macro, Scam Alert, Other]
- relevance: 0.0-1.0 how relevant the message is to each topic
- skip: true if message is greeting, spam, bot command, emoji-only, or has no informational value

Response format:
{"skip": false, "topics": [{"topic": "...", "topic_ko": "...", "category": "...", "relevance": 0.8}]}
If skip=true, topics array should be empty: {"skip": true, "topics": []}"#;

        let truncated = if text.len() > 500 { &text[..500] } else { text };
        let user = format!("[Channel: {}] [{}]\n{}", chat_title, timestamp, truncated);

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user,
                },
            ],
            temperature: 0.1,
            response_format: ResponseFormat {
                r#type: "json_object".to_string(),
            },
        };

        let response_text = self.call_api(&req).await?;
        serde_json::from_str::<ClassifyResponse>(&response_text).map_err(|e| {
            LlmError::Parse(format!(
                "Failed to parse classify response: {} - raw: {}",
                e, response_text
            ))
        })
    }

    pub async fn generate_summary(
        &self,
        title: &str,
        category: &str,
        source_messages: &[(usize, i64, &str, &str)],
    ) -> Result<(String, String), LlmError> {
        let system = r#"Write a bilingual wiki article about a crypto/finance topic based on Telegram messages.
Every factual claim MUST cite its source using [N] notation matching the message index.

Structure:
## 요약
(Korean summary: 2-3 paragraphs, factual, cite sources as [1], [2], etc.)

### 핵심 포인트
- (Korean bullet points with citations)

### 타임라인
- (Korean chronological events with citations)

---

## Summary
(English version of the same content with same citations)

### Key Points
- (English bullet points with citations)

### Timeline
- (English chronological events with citations)

Rules:
- EVERY factual claim must have a [N] citation to a source message
- If sources disagree, note the disagreement
- If information is unverified or speculative, mark it as such
- Skip duplicate forwarded messages
- If fewer than 3 unique source messages, output "Insufficient sources for wiki article"
- Keep it concise"#;

        let mut user = format!(
            "Topic: {}\nCategory: {}\n\nSource messages ({} total):\n",
            title,
            category,
            source_messages.len()
        );
        for &(idx, ts, chat_title, text) in source_messages {
            let truncated = if text.len() > 300 { &text[..300] } else { text };
            user.push_str(&format!(
                "[{}] [{}] [{}]: {}\n",
                idx, ts, chat_title, truncated
            ));
        }

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user,
                },
            ],
            temperature: 0.3,
            response_format: ResponseFormat {
                r#type: "text".to_string(),
            },
        };

        let response = self.call_api(&req).await?;
        let (ko, en) = split_bilingual(&response);
        Ok((ko, en))
    }

    pub async fn check_topic_dedup(
        &self,
        new_title: &str,
        existing_title: &str,
    ) -> Result<DedupResponse, LlmError> {
        let user = format!(
            "Are these the same crypto topic?\nNew: \"{}\"\nExisting: \"{}\"\nAnswer JSON: {{\"same\": true/false, \"confidence\": 0.0-1.0}}",
            new_title, existing_title
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user,
            }],
            temperature: 0.0,
            response_format: ResponseFormat {
                r#type: "json_object".to_string(),
            },
        };

        let response_text = self.call_api(&req).await?;
        serde_json::from_str::<DedupResponse>(&response_text).map_err(|e| {
            LlmError::Parse(format!("Dedup parse error: {} - raw: {}", e, response_text))
        })
    }

    async fn call_api(&self, req: &ChatRequest) -> Result<String, LlmError> {
        let resp = self
            .http
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(req)
            .send()
            .await
            .map_err(LlmError::Http)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!("{}: {}", status, body)));
        }

        let chat_resp: ChatResponse = resp.json().await.map_err(LlmError::Http)?;
        chat_resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| LlmError::Api("No choices in response".to_string()))
    }
}

fn split_bilingual(text: &str) -> (String, String) {
    if let Some(pos) = text.find("\n---\n") {
        let ko = text[..pos].trim().to_string();
        let en = text[pos + 5..].trim().to_string();
        return (ko, en);
    }

    if let Some(pos) = text.find("## Summary") {
        let ko = text[..pos].trim().to_string();
        let en = text[pos..].trim().to_string();
        return (ko, en);
    }

    (text.to_string(), text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_bilingual() {
        let text = "## 요약\n한국어 내용\n\n---\n\n## Summary\nEnglish content";
        let (ko, en) = split_bilingual(text);
        assert!(ko.contains("한국어"));
        assert!(en.contains("English"));
    }

    #[test]
    fn test_split_bilingual_no_separator() {
        let text = "## 요약\n한국어\n\n## Summary\nEnglish";
        let (ko, en) = split_bilingual(text);
        assert!(ko.contains("한국어"));
        assert!(en.contains("English"));
    }
}
