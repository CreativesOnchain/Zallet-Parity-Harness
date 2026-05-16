use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// A mock Zcash RPC node for testing.
pub struct MockNode {
    server: MockServer,
}

impl MockNode {
    /// Spawns a new mock node on a random port.
    pub async fn spawn() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    /// Returns the RPC URL for this node.
    pub fn url(&self) -> String {
        self.server.uri()
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Returns `true` if the request body matches `method_name` and `expected_params`.
    ///
    /// Param matching rules (aligned with `encode_params` in the client):
    /// - `expected_params = null` → accepts an empty params array (no-params call)
    /// - anything else           → the first element of the params array must equal `expected_params`
    fn matches_call(body: &Value, method_name: &str, expected_params: &Value) -> bool {
        let m = body.get("method").and_then(|v| v.as_str()) == Some(method_name);
        let params_arr = body.get("params").and_then(|v| v.as_array());
        let p = match (params_arr, expected_params) {
            // No-params call: empty array matches Value::Null expectation
            (None, Value::Null) | (Some(_), Value::Null) => {
                params_arr.is_none_or(|arr| arr.is_empty())
            }
            // Positional-params call: first element must equal the expectation
            (Some(arr), expected) => arr.first() == Some(expected),
            (None, _) => false,
        };
        m && p
    }

    // ── Mock helpers ─────────────────────────────────────────────────────────

    /// Mocks a successful JSON-RPC response for a specific method + params.
    pub async fn mock_response(&self, method_name: &str, expected_params: Value, result: Value) {
        let method_name = method_name.to_string();

        Mock::given(method("POST"))
            .and(path("/"))
            .and(move |req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                Self::matches_call(&body, &method_name, &expected_params)
            })
            .respond_with(move |req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                let id = body.get("id").cloned().unwrap_or(json!(1));
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0",
                    "result": result,
                    "id": id
                }))
            })
            .mount(&self.server)
            .await;
    }

    /// Mocks a JSON-RPC "method not found" response (error code -32601).
    /// Use this to test the MISSING classification path.
    pub async fn mock_method_not_found(&self, method_name: &str, expected_params: Value) {
        self.mock_rpc_error(method_name, expected_params, -32601, "Method not found")
            .await;
    }

    /// Mocks a generic JSON-RPC error response with the given code and message.
    /// Use this to test the ERROR classification path (non-missing errors).
    pub async fn mock_rpc_error(
        &self,
        method_name: &str,
        expected_params: Value,
        code: i32,
        error_message: &str,
    ) {
        let method_name = method_name.to_string();
        let error_message = error_message.to_string();

        Mock::given(method("POST"))
            .and(path("/"))
            .and(move |req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                Self::matches_call(&body, &method_name, &expected_params)
            })
            .respond_with(move |req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                let id = body.get("id").cloned().unwrap_or(json!(1));
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": code,
                        "message": error_message
                    },
                    "id": id
                }))
            })
            .mount(&self.server)
            .await;
    }

    /// Mocks a fallback response for all unhandled requests, returning Method Not Found.
    pub async fn mock_fallback(&self) {
        Mock::given(wiremock::matchers::any())
            .respond_with(|req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                let id = body.get("id").cloned().unwrap_or(json!(1));
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0",
                    "error": { "code": -32601, "message": "Method not found" },
                    "id": id
                }))
            })
            .mount(&self.server)
            .await;
    }
}
