//! Codex app-server requests that are not command/file approvals.
//!
//! Server requests are different from notifications: a turn may remain paused
//! until the client answers. Keep their wire shapes explicit so adding a new
//! request can never silently degrade into an ignored notification.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::protocol::{JsonRpcErrorBody, JsonRpcId};

pub const CHATGPT_AUTH_TOKENS_REFRESH: &str = "account/chatgptAuthTokens/refresh";
pub const ATTESTATION_GENERATE: &str = "attestation/generate";
pub const CURRENT_TIME_READ: &str = "currentTime/read";
pub const DYNAMIC_TOOL_CALL: &str = "item/tool/call";
pub const TOOL_REQUEST_USER_INPUT: &str = "item/tool/requestUserInput";
pub const MCP_SERVER_ELICITATION_REQUEST: &str = "mcpServer/elicitation/request";

/// Full Codex 0.147 server-request inventory, including approvals.
pub const SERVER_REQUEST_METHODS: &[&str] = &[
    CHATGPT_AUTH_TOKENS_REFRESH,
    "applyPatchApproval",
    ATTESTATION_GENERATE,
    CURRENT_TIME_READ,
    "execCommandApproval",
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/permissions/requestApproval",
    DYNAMIC_TOOL_CALL,
    TOOL_REQUEST_USER_INPUT,
    MCP_SERVER_ELICITATION_REQUEST,
];

pub fn is_known_server_request(method: &str) -> bool {
    SERVER_REQUEST_METHODS.contains(&method)
}

/// Requests safely answered in the transport pump without user interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum AutomaticServerResponse {
    Result(Value),
    Error(JsonRpcErrorBody),
}

pub fn automatic_server_response(
    method: &str,
    current_time_at: u64,
) -> Option<AutomaticServerResponse> {
    match method {
        CURRENT_TIME_READ => Some(AutomaticServerResponse::Result(json!({
            "currentTimeAt": current_time_at
        }))),
        DYNAMIC_TOOL_CALL => Some(AutomaticServerResponse::Result(json!({
            "success": false,
            "contentItems": [{
                "type": "inputText",
                "text": "This desktop did not register a client-owned dynamic tool for this call."
            }]
        }))),
        CHATGPT_AUTH_TOKENS_REFRESH => Some(AutomaticServerResponse::Error(unsupported_error(
            "This desktop uses Codex CLI authentication and does not manage ChatGPT access tokens.",
        ))),
        ATTESTATION_GENERATE => Some(AutomaticServerResponse::Error(unsupported_error(
            "Client attestation was not requested in initialize capabilities.",
        ))),
        _ => None,
    }
}

fn unsupported_error(message: &str) -> JsonRpcErrorBody {
    JsonRpcErrorBody {
        code: -32601,
        message: message.to_owned(),
        data: None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub options: Option<Vec<UserInputOption>>,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequestUserInputParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub is_blocking: bool,
    pub questions: Vec<UserInputQuestion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingUserInput {
    pub request_id: JsonRpcId,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub is_blocking: bool,
    pub questions: Vec<UserInputQuestion>,
}

impl PendingUserInput {
    pub fn response(answers: BTreeMap<String, Vec<String>>) -> Value {
        let answers = answers
            .into_iter()
            .map(|(id, answers)| (id, json!({ "answers": answers })))
            .collect::<serde_json::Map<_, _>>();
        json!({ "answers": answers })
    }
}

pub fn parse_user_input_request(
    id: JsonRpcId,
    method: &str,
    params: Option<&Value>,
) -> Option<PendingUserInput> {
    if method != TOOL_REQUEST_USER_INPUT {
        return None;
    }
    let parsed: ToolRequestUserInputParams = serde_json::from_value(params?.clone()).ok()?;
    Some(PendingUserInput {
        request_id: id,
        thread_id: parsed.thread_id,
        turn_id: parsed.turn_id,
        item_id: parsed.item_id,
        is_blocking: parsed.is_blocking,
        questions: parsed.questions,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpElicitationMode {
    Form { requested_schema: Value },
    OpenAiForm { requested_schema: Value },
    Url { elicitation_id: String, url: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingMcpElicitation {
    pub request_id: JsonRpcId,
    pub server_name: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub message: String,
    pub mode: McpElicitationMode,
}

impl PendingMcpElicitation {
    pub fn accept(content: Value) -> Value {
        json!({ "action": "accept", "content": content })
    }

    pub fn decline() -> Value {
        json!({ "action": "decline" })
    }

    pub fn cancel() -> Value {
        json!({ "action": "cancel" })
    }
}

pub fn parse_mcp_elicitation_request(
    id: JsonRpcId,
    method: &str,
    params: Option<&Value>,
) -> Option<PendingMcpElicitation> {
    if method != MCP_SERVER_ELICITATION_REQUEST {
        return None;
    }
    let params = params?;
    let text = |key: &str| params.get(key)?.as_str().map(str::to_owned);
    let mode = match text("mode")?.as_str() {
        "form" => McpElicitationMode::Form {
            requested_schema: params.get("requestedSchema")?.clone(),
        },
        "openai/form" => McpElicitationMode::OpenAiForm {
            requested_schema: params.get("requestedSchema")?.clone(),
        },
        "url" => McpElicitationMode::Url {
            elicitation_id: text("elicitationId")?,
            url: text("url")?,
        },
        _ => return None,
    };
    Some(PendingMcpElicitation {
        request_id: id,
        server_name: text("serverName")?,
        thread_id: text("threadId")?,
        turn_id: params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        message: text("message")?,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_noninteractive_requests_have_honest_responses() {
        let time = automatic_server_response(CURRENT_TIME_READ, 42).unwrap();
        assert_eq!(
            time,
            AutomaticServerResponse::Result(json!({"currentTimeAt": 42}))
        );

        let tool = automatic_server_response(DYNAMIC_TOOL_CALL, 0).unwrap();
        let AutomaticServerResponse::Result(tool) = tool else {
            panic!("dynamic tool result")
        };
        assert_eq!(tool["success"], false);
        assert!(tool["contentItems"][0]["text"]
            .as_str()
            .unwrap()
            .contains("did not register"));

        for method in [CHATGPT_AUTH_TOKENS_REFRESH, ATTESTATION_GENERATE] {
            assert!(matches!(
                automatic_server_response(method, 0),
                Some(AutomaticServerResponse::Error(_))
            ));
        }
    }

    #[test]
    fn generated_server_request_inventory_has_an_explicit_disposition() {
        let generated: Vec<_> = include_str!("../fixtures/server-requests.txt")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(generated, SERVER_REQUEST_METHODS);
        for method in generated {
            let classified = crate::approvals::is_approval_method(method)
                || matches!(
                    method,
                    TOOL_REQUEST_USER_INPUT | MCP_SERVER_ELICITATION_REQUEST
                )
                || automatic_server_response(method, 0).is_some();
            assert!(classified, "server request lacks disposition: {method}");
        }
    }

    #[test]
    fn parses_and_answers_user_input() {
        let params = json!({
            "threadId": "t1", "turnId": "u1", "itemId": "i1", "isBlocking": true,
            "questions": [{
                "id": "color", "header": "Color", "question": "Pick one",
                "options": [{"label": "Blue", "description": "Cool"}],
                "isOther": true, "isSecret": false
            }]
        });
        let pending =
            parse_user_input_request(JsonRpcId::Number(7), TOOL_REQUEST_USER_INPUT, Some(&params))
                .unwrap();
        assert_eq!(
            pending.questions[0].options.as_ref().unwrap()[0].label,
            "Blue"
        );
        let response = PendingUserInput::response(BTreeMap::from([(
            "color".to_owned(),
            vec!["Blue".to_owned()],
        )]));
        assert_eq!(response["answers"]["color"]["answers"][0], "Blue");
    }

    #[test]
    fn parses_mcp_url_elicitation() {
        let params = json!({
            "serverName": "example", "threadId": "t1", "turnId": "u1",
            "mode": "url", "elicitationId": "e1", "message": "Authorize",
            "url": "https://example.test/auth"
        });
        let pending = parse_mcp_elicitation_request(
            JsonRpcId::String("r1".into()),
            MCP_SERVER_ELICITATION_REQUEST,
            Some(&params),
        )
        .unwrap();
        assert!(matches!(pending.mode, McpElicitationMode::Url { .. }));
        assert_eq!(PendingMcpElicitation::decline()["action"], "decline");
    }
}
