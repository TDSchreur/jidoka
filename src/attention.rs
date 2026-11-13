//! Input and response boundaries in coordinator session logs. Conversation text is never retained.
use crate::usage_logs::timestamp;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Agent {
    Claude,
    Codex,
    Copilot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    /// An agent called its user-input tool.
    Requested,
    /// The agent finished a response; it may be waiting, or may have been interrupted.
    Finished,
    /// The user or the same session resumed work.
    Resumed,
}

#[derive(Default)]
struct Parser {
    pending: HashSet<String>,
    codex_sync: HashSet<String>,
    copilot_final: bool,
}

impl Parser {
    fn claude(&mut self, record: &Value) -> Option<Signal> {
        let message = &record["message"];
        match record["type"].as_str() {
            Some("assistant") => {
                let mut requested = false;
                if let Some(blocks) = message["content"].as_array() {
                    for block in blocks {
                        if block["type"] == "tool_use"
                            && block["name"] == "AskUserQuestion"
                            && let Some(id) = block["id"].as_str()
                        {
                            self.pending.insert(id.to_owned());
                            requested = true;
                        }
                    }
                }
                if requested {
                    Some(Signal::Requested)
                } else if message["stop_reason"] == "end_turn" && self.pending.is_empty() {
                    Some(Signal::Finished)
                } else {
                    None
                }
            }
            Some("user") => {
                if let Some(blocks) = message["content"].as_array() {
                    let answered = blocks.iter().any(|block| {
                        block["type"] == "tool_result"
                            && block["tool_use_id"]
                                .as_str()
                                .is_some_and(|id| self.pending.remove(id))
                    });
                    if blocks.iter().any(|block| block["type"] == "text") {
                        self.pending.clear();
                        Some(Signal::Resumed)
                    } else if answered && self.pending.is_empty() {
                        Some(Signal::Resumed)
                    } else {
                        None
                    }
                } else if message["content"].is_string() {
                    self.pending.clear();
                    Some(Signal::Resumed)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn codex(&mut self, record: &Value) -> Option<Signal> {
        let payload = &record["payload"];
        match (record["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("task_started")) => {
                self.pending.clear();
                self.codex_sync.clear();
                Some(Signal::Resumed)
            }
            (Some("event_msg"), Some("task_complete")) if self.pending.is_empty() => {
                Some(Signal::Finished)
            }
            (Some("response_item"), Some("function_call"))
                if matches!(
                    payload["name"].as_str(),
                    Some("request_user_input" | "request_user_input_async")
                ) =>
            {
                let id = payload["call_id"].as_str()?;
                self.pending.insert(id.to_owned());
                if payload["name"] == "request_user_input" {
                    self.codex_sync.insert(id.to_owned());
                }
                Some(Signal::Requested)
            }
            (Some("response_item"), Some("function_call_output")) => {
                if payload["call_id"]
                    .as_str()
                    .is_some_and(|id| self.codex_sync.remove(id) && self.pending.remove(id))
                    && self.pending.is_empty()
                {
                    Some(Signal::Resumed)
                } else {
                    None
                }
            }
            (Some("response_item"), Some("message")) if payload["role"] == "user" => {
                self.pending.clear();
                self.codex_sync.clear();
                Some(Signal::Resumed)
            }
            _ => None,
        }
    }

    fn copilot(&mut self, record: &Value) -> Option<Signal> {
        let data = &record["data"];
        // Copilot interleaves subagent turns in the coordinator transcript. Their events name
        // an agent or parent tool call; neither their questions nor their turns affect this session.
        if !record["agentId"].is_null()
            || !data["parentToolCallId"].is_null()
            || !data["parentAgentTaskId"].is_null()
        {
            return None;
        }
        match record["type"].as_str() {
            Some("tool.execution_start") if data["toolName"] == "ask_user" => {
                let id = data["toolCallId"].as_str()?;
                self.pending.insert(id.to_owned());
                Some(Signal::Requested)
            }
            Some("human_response.recorded") => {
                if data["toolCallId"]
                    .as_str()
                    .is_some_and(|id| self.pending.remove(id))
                    && self.pending.is_empty()
                {
                    Some(Signal::Resumed)
                } else {
                    None
                }
            }
            Some("user.message") if data["parentAgentTaskId"].is_null() => {
                self.pending.clear();
                self.copilot_final = false;
                Some(Signal::Resumed)
            }
            Some("assistant.message") if data["parentAgentTaskId"].is_null() => {
                self.copilot_final = data["content"].as_str().is_some_and(|s| !s.is_empty())
                    && data["toolRequests"].as_array().is_none_or(Vec::is_empty);
                None
            }
            Some("assistant.turn_end") if self.copilot_final && self.pending.is_empty() => {
                self.copilot_final = false;
                Some(Signal::Finished)
            }
            _ => None,
        }
    }
}

/// Extract signals from one coordinator transcript in log order. The caller must associate each
/// transcript with a run and keep sessions separate. Malformed and untimestamped records are ignored.
pub(crate) fn signals(agent: Agent, text: &str) -> Vec<(u64, Signal)> {
    let mut parser = Parser::default();
    text.lines()
        .filter_map(|line| {
            let record: Value = serde_json::from_str(line).ok()?;
            let at = timestamp(record.get("timestamp"))?;
            let signal = match agent {
                Agent::Claude => parser.claude(&record),
                Agent::Codex => parser.codex(&record),
                Agent::Copilot => parser.copilot(&record),
            }?;
            Some((at, signal))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn transcript(records: &[Value]) -> String {
        records
            .iter()
            .enumerate()
            .map(|(i, record)| {
                let mut record = record.clone();
                record["timestamp"] = json!(format!("2026-01-01T00:00:{i:02}Z"));
                record.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn kinds(agent: Agent, records: &[Value]) -> Vec<Signal> {
        signals(agent, &transcript(records))
            .into_iter()
            .map(|(_, signal)| signal)
            .collect()
    }

    #[test]
    fn claude_requests_responses_and_unrelated_tool_results() {
        assert_eq!(
            kinds(
                Agent::Claude,
                &[
                    json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"AskUserQuestion","id":"a"}]}}),
                    json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"other"}]}}),
                    json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a"}]}}),
                    json!({"type":"assistant","message":{"stop_reason":"end_turn"}}),
                    json!({"type":"user","message":{"content":"continue"}}),
                ]
            ),
            [
                Signal::Requested,
                Signal::Resumed,
                Signal::Finished,
                Signal::Resumed
            ]
        );
    }

    #[test]
    fn codex_async_tool_output_does_not_resume() {
        assert_eq!(
            kinds(
                Agent::Codex,
                &[
                    json!({"type":"event_msg","payload":{"type":"task_started"}}),
                    json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input_async","call_id":"a"}}),
                    json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a"}}),
                    json!({"type":"event_msg","payload":{"type":"task_complete"}}),
                    json!({"type":"event_msg","payload":{"type":"task_started"}}),
                    json!({"type":"event_msg","payload":{"type":"task_complete"}}),
                ]
            ),
            [
                Signal::Resumed,
                Signal::Requested,
                Signal::Resumed,
                Signal::Finished
            ]
        );
    }

    #[test]
    fn codex_matching_synchronous_answer_and_user_message_resume() {
        assert_eq!(
            kinds(
                Agent::Codex,
                &[
                    json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input","call_id":"a"}}),
                    json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"other"}}),
                    json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a"}}),
                    json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input_async","call_id":"b"}}),
                    json!({"type":"response_item","payload":{"type":"message","role":"user"}}),
                ]
            ),
            [
                Signal::Requested,
                Signal::Resumed,
                Signal::Requested,
                Signal::Resumed
            ]
        );
    }

    #[test]
    fn copilot_matches_human_response_and_final_turn() {
        assert_eq!(
            kinds(
                Agent::Copilot,
                &[
                    json!({"type":"tool.execution_start","data":{"toolName":"ask_user","toolCallId":"a"}}),
                    json!({"type":"human_response.recorded","data":{"toolCallId":"other"}}),
                    json!({"type":"human_response.recorded","data":{"toolCallId":"a"}}),
                    json!({"type":"assistant.message","data":{"content":"question?","toolRequests":[]}}),
                    json!({"type":"assistant.turn_end","data":{}}),
                    json!({"type":"user.message","data":{"content":"answer"}}),
                ]
            ),
            [
                Signal::Requested,
                Signal::Resumed,
                Signal::Finished,
                Signal::Resumed
            ]
        );
    }

    #[test]
    fn copilot_subagent_events_do_not_change_coordinator_attention() {
        assert_eq!(
            kinds(
                Agent::Copilot,
                &[
                    json!({"type":"tool.execution_start","data":{"toolName":"ask_user","toolCallId":"coordinator"}}),
                    json!({"type":"user.message","data":{"parentAgentTaskId":"subagent","content":"continue"}}),
                    json!({"type":"tool.execution_start","agentId":"subagent","data":{"toolName":"ask_user","toolCallId":"sub"}}),
                    json!({"type":"assistant.message","agentId":"subagent","data":{"parentToolCallId":"parent","content":"done","toolRequests":[]}}),
                    json!({"type":"assistant.turn_end","agentId":"subagent","data":{"parentToolCallId":"parent"}}),
                    json!({"type":"human_response.recorded","data":{"toolCallId":"sub"}}),
                    json!({"type":"human_response.recorded","data":{"toolCallId":"coordinator"}}),
                    json!({"type":"assistant.message","data":{"content":"done","toolRequests":[]}}),
                    json!({"type":"assistant.turn_end","data":{"parentToolCallId":"parent"}}),
                    json!({"type":"assistant.turn_end","data":{}}),
                ]
            ),
            [Signal::Requested, Signal::Resumed, Signal::Finished]
        );
    }

    #[test]
    fn ignores_bad_lines_and_unfinished_copilot_messages() {
        assert!(signals(Agent::Codex, "bad json\n{\"type\":\"event_msg\"}").is_empty());
        assert!(kinds(Agent::Copilot, &[
            json!({"type":"assistant.message","data":{"content":"working","toolRequests":[{"name":"shell"}]}}),
            json!({"type":"assistant.turn_end","data":{}}),
        ]).is_empty());
    }
}
