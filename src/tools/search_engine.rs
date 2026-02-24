//! SearxNG-backed search tool.
//!
//! Implements a simple search tool that queries a configured SearxNG
//! instance's JSON API and returns a summary plus structured results.

use async_trait::async_trait;
use reqwest::Client;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use super::{Tool, ToolContext, ToolOutput};
use crate::error::{Result, ZeptoError};

const DEFAULT_USER_AGENT: &str = "zeptoclaw/0.1 (+https://github.com/zeptoclaw/zeptoclaw)";
const DEFAULT_TIMEOUT_SECS: u64 = 15;

/// SearxNG search tool.
pub struct SearxSearchTool {
    base_url: String,
    api_key: Option<String>,
    client: Client,
    max_results: usize,
}

impl SearxSearchTool {
    pub fn new(base_url: &str, api_key: Option<&str>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.map(|s| s.to_string()),
            client,
            max_results: 6,
        }
    }

    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max.clamp(1, 50);
        self
    }
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct SearxResult {
    title: String,
    url: String,
    #[serde(alias = "content")]
    content: Option<String>,
    #[serde(alias = "snippet")]
    snippet: Option<String>,
    engine: Option<String>,
    #[serde(rename = "publishedDate", alias = "date")]
    published_date: Option<String>,
    #[serde(rename = "parsed_url")]
    parsed_url: Option<Vec<String>>,
    #[serde(rename = "img_src")]
    img_src: Option<String>,
    thumbnail: Option<String>,
    priority: Option<String>,
    engines: Option<Vec<String>>,
    positions: Option<Vec<u32>>,
    score: Option<f64>,
    category: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct SearxPagination {
    pageno: Option<u32>,
    per_page: Option<u32>,
    total_results: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct SearxResponse {
    #[serde(default)]
    results: Vec<SearxResult>,
    #[serde(default)]
    pagination: Option<SearxPagination>,
    #[serde(rename = "number_of_results")]
    number_of_results: Option<u32>,
}

#[async_trait]
impl Tool for SearxSearchTool {
    fn name(&self) -> &str {
        "search_engine"
    }

    fn description(&self) -> &str {
        "Search a configured SearxNG instance and return titles, URLs and snippets."
    }

    fn compact_description(&self) -> &str {
        "Search (SearxNG)"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "q": { "type": "string", "description": "Search query" },
                "engines": { "type": "string", "description": "Comma-separated engine names" },
                "categories": { "type": "string", "description": "Comma-separated categories" },
                "lang": { "type": "string", "description": "Language code (e.g., en)" },
                "pageno": { "type": "integer", "description": "Page number (1-based)" },
                "max_results": { "type": "integer", "description": "Maximum results to include in summary" }
            },
            "required": ["q"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let query = args
            .get("q")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ZeptoError::Tool("Missing 'q' parameter".to_string()))?;

        let engines = args.get("engines").and_then(|v| v.as_str()).map(str::trim);
        let categories = args
            .get("categories")
            .and_then(|v| v.as_str())
            .map(str::trim);
        let lang = args.get("lang").and_then(|v| v.as_str()).map(str::trim);
        let pageno = args
            .get("pageno")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(1);
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.max_results)
            .clamp(1, 50);

        if self.base_url.trim().is_empty() {
            return Err(ZeptoError::Tool(
                "Searx base_url not configured".to_string(),
            ));
        }

        // Build search URL (append /search)
        let url_str = format!("{}/search", self.base_url.trim_end_matches('/'));
        let mut url = Url::parse(&url_str)
            .map_err(|e| ZeptoError::Tool(format!("Invalid base_url: {}", e)))?;

        // Build query parameters
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("q", query);
            qp.append_pair("format", "json");
            qp.append_pair("pageno", &pageno.to_string());
            if let Some(e) = engines {
                if !e.is_empty() {
                    qp.append_pair("engines", e);
                }
            }
            if let Some(c) = categories {
                if !c.is_empty() {
                    qp.append_pair("categories", c);
                }
            }
            if let Some(l) = lang {
                if !l.is_empty() {
                    qp.append_pair("lang", l);
                }
            }
        }

        let mut req = self
            .client
            .get(url.clone())
            .header("Accept", "application/json")
            .header("User-Agent", DEFAULT_USER_AGENT);

        if let Some(k) = &self.api_key {
            if !k.trim().is_empty() {
                req = req.header("Authorization", k.as_str());
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ZeptoError::Tool(format!("Search request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.trim();
            return Err(ZeptoError::Tool(if detail.is_empty() {
                format!("Searx API error: {}", status)
            } else {
                format!("Searx API error: {} ({})", status, detail)
            }));
        }

        // Parse raw JSON so we can inspect `unresponsive_engines` and prune fields.
        let raw: Value = resp
            .json()
            .await
            .map_err(|e| ZeptoError::Tool(format!("Failed to parse searx response: {}", e)))?;

        // Build pruned JSON value according to tailoring rules (remove low-value fields,
        // filter results from unresponsive engines). We'll then serialize according to
        // configuration (`markdown` by default).
        let pruned = build_pruned_output(query, &raw, max_results);

        // Load config to determine output format (default: markdown)
        let cfg = crate::config::Config::load().unwrap_or_default();
        let format = cfg.tools.search_engine.format.as_str();

        if format == "json" {
            let s = serde_json::to_string(&pruned)
                .map_err(|e| ZeptoError::Tool(format!("Failed to serialize json: {}", e)))?;
            Ok(ToolOutput::user_visible(s))
        } else {
            // Default: markdown
            let md = build_search_markdown(query, &pruned, max_results);
            Ok(ToolOutput::user_visible(md))
        }
    }
}
/// Build a pruned JSON `Value` from the raw Searx response `raw`.
/// Removes low-value fields and filters out results from `unresponsive_engines`.
fn build_pruned_output(_query: &str, raw: &Value, _max_results: usize) -> serde_json::Value {
    // Clone raw so we can mutate
    let mut out = raw.clone();

    // Determine unresponsive engine names
    let mut unresponsive: Vec<String> = Vec::new();
    if let Some(arr) = raw.get("unresponsive_engines").and_then(|v| v.as_array()) {
        for item in arr.iter() {
            if let Some(name) = item.get(0).and_then(|v| v.as_str()) {
                unresponsive.push(name.to_string());
            }
        }
    }

    // Replace `results` with a pruned+filtered array
    if let Some(results_arr) = raw.get("results").and_then(|v| v.as_array()) {
        let mut pruned_results: Vec<Value> = Vec::new();

        for r in results_arr.iter() {
            // Skip if engine is unresponsive
            if let Some(engine) = r.get("engine").and_then(|v| v.as_str()) {
                if unresponsive.iter().any(|e| e == engine) {
                    continue;
                }
            }

            // Build a new object only with allowed fields
            if let Some(obj) = r.as_object() {
                let mut map = serde_json::Map::new();

                if let Some(v) = obj.get("url") {
                    map.insert("url".to_string(), v.clone());
                }
                if let Some(v) = obj.get("title") {
                    map.insert("title".to_string(), v.clone());
                }
                if let Some(v) = obj.get("content") {
                    map.insert("content".to_string(), v.clone());
                }
                if let Some(v) = obj.get("snippet") {
                    map.insert("snippet".to_string(), v.clone());
                }
                if let Some(v) = obj.get("publishedDate") {
                    map.insert("publishedDate".to_string(), v.clone());
                }
                if let Some(v) = obj.get("img_src") {
                    map.insert("img_src".to_string(), v.clone());
                }
                if let Some(v) = obj.get("score") {
                    map.insert("score".to_string(), v.clone());
                }

                pruned_results.push(Value::Object(map));
            }
        }

        if let Some(m) = out.as_object_mut() {
            m.insert("results".to_string(), Value::Array(pruned_results));
            // Remove removed top-level keys
            m.remove("number_of_results");
            m.remove("unresponsive_engines");
        }
    }

    out
}

/// Render the pruned JSON value as a Markdown string. Ensures every field present
/// in the JSON is represented in the Markdown output (content is kept verbatim).
fn build_search_markdown(_query: &str, pruned: &Value, max_results: usize) -> String {
    let mut out = String::new();

    let query = pruned.get("query").and_then(|v| v.as_str()).unwrap_or("");
    out.push_str(&format!("# search: {}\n\n", query));

    let empty: Vec<Value> = Vec::new();
    let results = pruned
        .get("results")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    out.push_str(&format!("## Results ({})\n\n", results.len()));

    for (i, r) in results.iter().take(max_results).enumerate() {
        let title = r
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(no title)");
        let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
        out.push_str(&format!("{}. **{}** — {}\n\n", i + 1, title, url));

        // Full content block (verbatim)
        if let Some(content) = r.get("content").and_then(|v| v.as_str()) {
            out.push_str(&format!("{}\n\n", content));
        }

        // Render any other present fields (snippet, publishedDate, img_src, score)
        if let Some(snippet) = r.get("snippet").and_then(|v| v.as_str()) {
            out.push_str(&format!("**snippet**: {}\n\n", snippet));
        }
        if let Some(pd) = r.get("publishedDate").and_then(|v| v.as_str()) {
            out.push_str(&format!("**publishedDate**: {}\n\n", pd));
        }
        if let Some(img) = r.get("img_src").and_then(|v| v.as_str()) {
            if !img.is_empty() {
                out.push_str(&format!("**img_src**: {}\n\n", img));
            }
        }
        if let Some(score) = r.get("score") {
            out.push_str(&format!("**score**: {}\n\n", score));
        }
    }

    // Suggestions
    if let Some(suggestions) = pruned.get("suggestions").and_then(|v| v.as_array()) {
        if !suggestions.is_empty() {
            out.push_str("### Suggestions\n\n");
            for s in suggestions.iter() {
                if let Some(text) = s.as_str() {
                    out.push_str(&format!("- {}\n", text));
                }
            }
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parameters_schema() {
        let tool = SearxSearchTool::new("https://example.com", None);
        let params = tool.parameters();
        assert!(params.is_object());
        assert!(params["properties"].is_object());
    }

    #[tokio::test]
    async fn test_name_and_description() {
        let tool = SearxSearchTool::new("https://example.com", None);
        assert_eq!(tool.name(), "search_engine");
        assert!(tool.description().len() > 0);
    }

    #[test]
    fn test_deserialize_fixture_with_number_of_results_zero() {
        // Fixture: `number_of_results` is 0 but `results` array contains entries.
        let fixture = r#"
{
  "query": "openclaw latest features",
  "number_of_results": 0,
  "results": [
    {
      "url": "https://openclaw.ai/",
      "title": "OpenClaw — Personal AI Assistant",
      "content": "The AI that actually does things. Clears your inbox, sends emails, manages your calendar, checks you in for flights. All from WhatsApp, Telegram, or any chat ...",
      "thumbnail": null,
      "engine": "google",
      "template": "default.html",
      "parsed_url": [
        "https",
        "openclaw.ai",
        "/",
        "",
        "",
        ""
      ],
      "img_src": "",
      "priority": "",
      "engines": [
        "google"
      ],
      "positions": [1],
      "score": 1,
      "category": "general"
    }
  ],
  "answers": [],
  "corrections": [],
  "infoboxes": [],
  "suggestions": [
    "How do i open clawdbot",
    "Clawdbot hub skills",
    "Clawdbot how to start",
    "Openclaw latest features ios",
    "ClawdBot installation instructions",
    "Moltbot showcase",
    "Clawedbot GitHub",
    "Openclaw latest features reddit"
  ],
  "unresponsive_engines": [
    [
      "brave",
      "timeout"
    ],
    [
      "startpage",
      "Suspended: timeout"
    ],
    [
      "wikidata",
      "Suspended: timeout"
    ]
  ]
}"#;

        // Parse raw fixture into Value so we can exercise pruning logic.
        let raw: Value = serde_json::from_str(fixture).expect("should parse fixture");
        // Sanity: original raw has results
        let raw_results = raw
            .get("results")
            .and_then(|v| v.as_array())
            .expect("fixture must have results");
        assert!(
            !raw_results.is_empty(),
            "results should be preserved even if number_of_results is zero"
        );

        // Build pruned output
        let pruned = build_pruned_output("openclaw latest features", &raw, 6);

        let results = pruned
            .get("results")
            .and_then(|v| v.as_array())
            .expect("pruned must have results");
        assert!(!results.is_empty());

        let r0 = &results[0];
        assert_eq!(
            r0.get("title").and_then(|v| v.as_str()),
            Some("OpenClaw — Personal AI Assistant")
        );
        assert_eq!(
            r0.get("url").and_then(|v| v.as_str()),
            Some("https://openclaw.ai/")
        );
        assert!(r0
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .starts_with("The AI that actually does things."));

        // Removed fields should be absent
        let removed = [
            "thumbnail",
            "engine",
            "template",
            "parsed_url",
            "priority",
            "engines",
            "positions",
            "category",
        ];
        for key in removed.iter() {
            assert!(
                r0.get(*key).is_none(),
                "{} must be removed from results",
                key
            );
        }

        // Top-level `number_of_results` and `unresponsive_engines` must be removed
        assert!(pruned.get("number_of_results").is_none());
        assert!(pruned.get("unresponsive_engines").is_none());

        // Suggestions preserved
        assert!(pruned
            .get("suggestions")
            .and_then(|v| v.as_array())
            .is_some());
    }
}
