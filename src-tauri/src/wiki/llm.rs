use serde::Deserialize;
use std::process::Command;

/// LLM client that shells out to `codex exec` CLI.
/// No API keys or OAuth needed — uses whatever auth codex has configured.
#[derive(Debug, Clone)]
pub struct LlmClient;

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
    Exec(String),
    Parse(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Exec(e) => write!(f, "Codex exec error: {}", e),
            LlmError::Parse(e) => write!(f, "Parse error: {}", e),
        }
    }
}

impl std::error::Error for LlmError {}

/// Check if `codex` CLI is available on PATH.
pub fn is_codex_available() -> bool {
    Command::new("codex")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run a prompt through `codex exec` and return the text output.
fn run_codex(prompt: &str) -> Result<String, LlmError> {
    let output_file =
        std::env::temp_dir().join(format!("tg-wiki-codex-{}.txt", std::process::id()));

    let result = Command::new("codex")
        .args([
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "-o",
            output_file.to_str().unwrap_or("/tmp/tg-wiki-codex.txt"),
            prompt,
        ])
        .output()
        .map_err(|e| LlmError::Exec(format!("Failed to run codex: {}", e)))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        return Err(LlmError::Exec(format!(
            "codex exec failed: {}{}",
            stderr, stdout
        )));
    }

    // Read from output file (cleaner than parsing stdout which has metadata)
    let text = std::fs::read_to_string(&output_file)
        .map_err(|e| LlmError::Exec(format!("Failed to read codex output: {}", e)))?;
    let _ = std::fs::remove_file(&output_file);

    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err(LlmError::Exec("Empty response from codex".to_string()));
    }

    Ok(trimmed)
}

/// Async wrapper — runs codex in a blocking task to not block tokio.
async fn run_codex_async(prompt: String) -> Result<String, LlmError> {
    tokio::task::spawn_blocking(move || run_codex(&prompt))
        .await
        .map_err(|e| LlmError::Exec(format!("Task join error: {}", e)))?
}

impl Default for LlmClient {
    fn default() -> Self {
        Self
    }
}

impl LlmClient {
    pub fn new() -> Self {
        Self
    }

    pub async fn validate(&self) -> Result<bool, LlmError> {
        match run_codex_async("Reply with ONLY the word ok".to_string()).await {
            Ok(text) => Ok(text.contains("ok")),
            Err(_) => Ok(false),
        }
    }

    pub async fn classify_message(
        &self,
        chat_title: &str,
        timestamp: i64,
        text: &str,
    ) -> Result<ClassifyResponse, LlmError> {
        let truncated = if text.len() > 500 { &text[..500] } else { text };

        let prompt = format!(
            r#"You are a crypto/finance message classifier. Classify this Telegram message into topics. Return ONLY valid JSON, no other text.

Rules:
- topics: array of 1-3 topics
- Each topic: concise English title
- topic_ko: Korean title if inferrable, else null
- category: one of [DeFi, Trading, L1/L2, NFT, Airdrop, Regulation, Macro, Scam Alert, Other]
- relevance: 0.0-1.0
- skip: true if greeting, spam, bot command, emoji-only, no info value

Message: [Channel: {}] [{}]
{}

Return JSON: {{"skip": false, "topics": [{{"topic": "...", "topic_ko": "...", "category": "...", "relevance": 0.8}}]}}
If skip: {{"skip": true, "topics": []}}"#,
            chat_title, timestamp, truncated
        );

        let response = run_codex_async(prompt).await?;

        // Extract JSON from response (codex might add extra text)
        let json_str = extract_json(&response)
            .ok_or_else(|| LlmError::Parse(format!("No JSON found in: {}", response)))?;

        serde_json::from_str::<ClassifyResponse>(json_str)
            .map_err(|e| LlmError::Parse(format!("JSON parse error: {} — raw: {}", e, json_str)))
    }

    pub async fn generate_summary(
        &self,
        title: &str,
        category: &str,
        source_messages: &[(usize, i64, &str, &str)],
    ) -> Result<(String, String), LlmError> {
        let mut sources_text = String::new();
        for &(idx, ts, chat_title, text) in source_messages {
            let truncated = if text.len() > 300 { &text[..300] } else { text };
            sources_text.push_str(&format!(
                "[{}] [{}] [{}]: {}\n",
                idx, ts, chat_title, truncated
            ));
        }

        let prompt = format!(
            r#"Write a bilingual wiki article about a crypto/finance topic. Every claim MUST cite sources using [N].

Structure:
## 요약
(Korean, 2-3 paragraphs with [N] citations)
### 핵심 포인트
- (Korean bullets with citations)
### 타임라인
- (Korean events with citations)
---
## Summary
(English version with same [N] citations)
### Key Points
- (English bullets)
### Timeline
- (English events)

Rules: Every claim needs [N] citation. Note disagreements. Mark unverified info. Skip duplicates. If <3 sources, say "Insufficient sources."

Topic: {}
Category: {}

Source messages ({} total):
{}"#,
            title,
            category,
            source_messages.len(),
            sources_text
        );

        let response = run_codex_async(prompt).await?;
        let (ko, en) = split_bilingual(&response);
        Ok((ko, en))
    }

    pub async fn check_topic_dedup(
        &self,
        new_title: &str,
        existing_title: &str,
    ) -> Result<DedupResponse, LlmError> {
        let prompt = format!(
            r#"Are these the same crypto topic? Reply with ONLY JSON.
New: "{}"
Existing: "{}"
Reply: {{"same": true/false, "confidence": 0.0-1.0}}"#,
            new_title, existing_title
        );

        let response = run_codex_async(prompt).await?;
        let json_str = extract_json(&response)
            .ok_or_else(|| LlmError::Parse(format!("No JSON found in: {}", response)))?;

        serde_json::from_str::<DedupResponse>(json_str)
            .map_err(|e| LlmError::Parse(format!("Dedup parse error: {} — raw: {}", e, json_str)))
    }
}

/// Extract the first JSON object from a string (codex may add surrounding text).
fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0;
    for (i, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
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
    fn test_extract_json() {
        assert_eq!(
            extract_json(r#"Here is the result: {"skip": true, "topics": []}"#),
            Some(r#"{"skip": true, "topics": []}"#)
        );
    }

    #[test]
    fn test_extract_json_nested() {
        let text = r#"{"topics": [{"topic": "ETH", "relevance": 0.9}]}"#;
        assert_eq!(extract_json(text), Some(text));
    }

    #[test]
    fn test_extract_json_none() {
        assert_eq!(extract_json("no json here"), None);
    }

    #[test]
    fn test_split_bilingual() {
        let text = "## 요약\n한국어 내용\n\n---\n\n## Summary\nEnglish content";
        let (ko, en) = split_bilingual(text);
        assert!(ko.contains("한국어"));
        assert!(en.contains("English"));
    }
}
