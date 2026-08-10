//! Typed Codex app-server realtime voice contracts.
//!
//! Mitsuro HTTP does not currently expose an equivalent transport. Callers must
//! gate this module through [`crate::BackendCapabilities::realtime_voice`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::LifecycleNotification;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RealtimeConversationVersion {
    V1,
    V2,
    V3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RealtimeOutputModality {
    Text,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationTextRole {
    User,
    Developer,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexResponseHandoffMode {
    Thinking,
    Commentary,
    BemTags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RealtimeVoice {
    Alloy,
    Arbor,
    Ash,
    Ballad,
    Breeze,
    Cedar,
    Coral,
    Cove,
    Echo,
    Ember,
    Juniper,
    Maple,
    Marin,
    Sage,
    Shimmer,
    Sol,
    Spruce,
    Vale,
    Verse,
}

impl RealtimeVoice {
    pub fn id(self) -> &'static str {
        match self {
            Self::Alloy => "alloy",
            Self::Arbor => "arbor",
            Self::Ash => "ash",
            Self::Ballad => "ballad",
            Self::Breeze => "breeze",
            Self::Cedar => "cedar",
            Self::Coral => "coral",
            Self::Cove => "cove",
            Self::Echo => "echo",
            Self::Ember => "ember",
            Self::Juniper => "juniper",
            Self::Maple => "maple",
            Self::Marin => "marin",
            Self::Sage => "sage",
            Self::Shimmer => "shimmer",
            Self::Sol => "sol",
            Self::Spruce => "spruce",
            Self::Vale => "vale",
            Self::Verse => "verse",
        }
    }

    pub fn label(self) -> String {
        let id = self.id();
        let mut chars = id.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeVoicesList {
    pub default_v1: RealtimeVoice,
    pub default_v2: RealtimeVoice,
    pub v1: Vec<RealtimeVoice>,
    pub v2: Vec<RealtimeVoice>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeListVoicesParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeListVoicesResponse {
    pub voices: RealtimeVoicesList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ThreadRealtimeStartTransport {
    Websocket,
    Webrtc { sdp: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeInitialItem {
    pub role: ConversationTextRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRealtimeStartParams {
    pub thread_id: String,
    pub output_modality: RealtimeOutputModality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<ThreadRealtimeStartTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<RealtimeConversationVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<RealtimeVoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_items: Option<Vec<ThreadRealtimeInitialItem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_startup_context: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_transcript_tail_on_session_end: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime_start_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime_end_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_ack_filler: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_managed_handoffs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_response_handoff_mode: Option<CodexResponseHandoffMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_response_handoff_channel_prefixes: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_responses_as_items: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_response_item_prefix: Option<String>,
}

impl ThreadRealtimeStartParams {
    pub fn websocket(
        thread_id: impl Into<String>,
        output_modality: RealtimeOutputModality,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            output_modality,
            model: None,
            prompt: None,
            transport: Some(ThreadRealtimeStartTransport::Websocket),
            version: Some(RealtimeConversationVersion::V3),
            voice: None,
            realtime_session_id: None,
            initial_items: None,
            include_startup_context: Some(true),
            flush_transcript_tail_on_session_end: Some(true),
            realtime_start_instructions: None,
            realtime_end_instructions: None,
            delegation_ack_filler: None,
            client_managed_handoffs: None,
            codex_response_handoff_mode: None,
            codex_response_handoff_channel_prefixes: None,
            codex_responses_as_items: None,
            codex_response_item_prefix: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeStartResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRealtimeAudioChunk {
    pub data: String,
    pub num_channels: u16,
    pub sample_rate: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples_per_channel: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRealtimeAppendAudioParams {
    pub thread_id: String,
    pub audio: ThreadRealtimeAudioChunk,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeAppendAudioResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRealtimeAppendTextParams {
    pub thread_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ConversationTextRole>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeAppendTextResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRealtimeAppendSpeechParams {
    pub thread_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeAppendSpeechResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRealtimeStopParams {
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRealtimeStopResponse {}

#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeEvent {
    Started {
        thread_id: String,
        version: RealtimeConversationVersion,
        realtime_session_id: Option<String>,
    },
    TranscriptDelta {
        thread_id: String,
        role: String,
        delta: String,
    },
    TranscriptDone {
        thread_id: String,
        role: String,
        text: String,
    },
    OutputAudio {
        thread_id: String,
        audio: ThreadRealtimeAudioChunk,
    },
    ItemAdded {
        thread_id: String,
        item: Value,
    },
    Error {
        thread_id: String,
        message: String,
    },
    Closed {
        thread_id: String,
        reason: Option<String>,
    },
    Sdp {
        thread_id: String,
        sdp: String,
    },
}

impl RealtimeEvent {
    pub fn from_lifecycle(event: &LifecycleNotification) -> Option<Self> {
        let params = event.params.as_ref()?;
        let thread_id = params.get("threadId")?.as_str()?.to_owned();
        Some(match event.method.as_str() {
            "thread/realtime/started" => Self::Started {
                thread_id,
                version: serde_json::from_value(params.get("version")?.clone()).ok()?,
                realtime_session_id: params
                    .get("realtimeSessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            "thread/realtime/transcript/delta" => Self::TranscriptDelta {
                thread_id,
                role: params.get("role")?.as_str()?.to_owned(),
                delta: params.get("delta")?.as_str()?.to_owned(),
            },
            "thread/realtime/transcript/done" => Self::TranscriptDone {
                thread_id,
                role: params.get("role")?.as_str()?.to_owned(),
                text: params.get("text")?.as_str()?.to_owned(),
            },
            "thread/realtime/outputAudio/delta" => Self::OutputAudio {
                thread_id,
                audio: serde_json::from_value(params.get("audio")?.clone()).ok()?,
            },
            "thread/realtime/itemAdded" => Self::ItemAdded {
                thread_id,
                item: params.get("item")?.clone(),
            },
            "thread/realtime/error" => Self::Error {
                thread_id,
                message: params
                    .get("message")
                    .or_else(|| params.get("error"))
                    .and_then(|value| value.as_str().or_else(|| value.get("message")?.as_str()))
                    .unwrap_or("Realtime session failed")
                    .to_owned(),
            },
            "thread/realtime/closed" => Self::Closed {
                thread_id,
                reason: params
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            "thread/realtime/sdp" => Self::Sdp {
                thread_id,
                sdp: params.get("sdp")?.as_str()?.to_owned(),
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LifecycleNotification;

    #[test]
    fn start_and_audio_params_match_generated_camel_case_contract() {
        let mut start =
            ThreadRealtimeStartParams::websocket("thread-1", RealtimeOutputModality::Audio);
        start.voice = Some(RealtimeVoice::Sol);
        let value = serde_json::to_value(start).expect("serialize start");
        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["outputModality"], "audio");
        assert_eq!(value["transport"]["type"], "websocket");
        assert_eq!(value["version"], "v3");
        assert_eq!(value["voice"], "sol");

        let value = serde_json::to_value(ThreadRealtimeAppendAudioParams {
            thread_id: "thread-1".to_owned(),
            audio: ThreadRealtimeAudioChunk {
                data: "AQI=".to_owned(),
                num_channels: 1,
                sample_rate: 24_000,
                samples_per_channel: Some(1),
                item_id: None,
            },
        })
        .expect("serialize audio");
        assert_eq!(value["audio"]["numChannels"], 1);
        assert_eq!(value["audio"]["sampleRate"], 24_000);
        assert_eq!(value["audio"]["samplesPerChannel"], 1);
    }

    #[test]
    fn lifecycle_notifications_parse_into_typed_realtime_events() {
        let lifecycle = LifecycleNotification::from_known(
            "thread/realtime/transcript/done",
            Some(&serde_json::json!({
                "threadId": "thread-7",
                "role": "user",
                "text": "hello world"
            })),
        )
        .expect("known notification");
        assert_eq!(
            RealtimeEvent::from_lifecycle(&lifecycle),
            Some(RealtimeEvent::TranscriptDone {
                thread_id: "thread-7".to_owned(),
                role: "user".to_owned(),
                text: "hello world".to_owned(),
            })
        );
    }
}
