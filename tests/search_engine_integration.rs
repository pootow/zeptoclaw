use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeptoclaw::tools::Tool;

#[tokio::test]
async fn searx_search_returns_summary_and_results() {
    // Start a mock server
    let server = MockServer::start().await;

    // Prepare a fake searx JSON response
    let body = json!({
        "results": [
            {
                "title": "Rust Programming Language",
                "url": "https://www.rust-lang.org/",
                "snippet": "Rust is a systems programming language...",
                "engine": "bing",
                "date": "2024-01-01T00:00:00Z"
            }
        ],
        "pagination": { "pageno": 1, "per_page": 10, "total_results": 1 }
    });

    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    // Construct tool pointing at mock server
    let base = server.uri();

    let tool = zeptoclaw::tools::SearxSearchTool::new(&base, None);
    let args = json!({ "q": "rust" });

    let ctx = zeptoclaw::tools::ToolContext::new();
    let res = tool.execute(args, &ctx).await.expect("tool should succeed");

    // Response should be JSON with summary and results (tool returns ToolOutput)
    let v: serde_json::Value = serde_json::from_str(&res.for_llm).expect("valid json");
    assert!(v.get("summary").is_some());
    let results = v
        .get("results")
        .and_then(|r| r.as_array())
        .expect("results array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["title"], "Rust Programming Language");
}
