//! `Conversation` — bidirectional voice session built on top of the
//! [`atomr_agents_stt_core::SpeechToText`] and
//! [`atomr_agents_tts_core::TextToSpeech`] traits.
//!
//! Two operating modes:
//!
//! - [`ConversationMode::TurnBased`] — run STT on caller-supplied
//!   PCM, hand the transcript to the user-supplied
//!   [`ConversationAgent`], synthesise the reply via TTS. Works with
//!   any STT + TTS pair.
//! - [`ConversationMode::UnifiedRealtime`] — open one
//!   [`atomr_agents_tts_core::RealtimeSession`] (e.g. OpenAI Realtime,
//!   Gemini Live, ElevenLabs Conversational AI) that both transcribes
//!   inbound audio and emits assistant audio. The agent is consulted
//!   only if the backend surfaces an [`InboundTranscript`] event and
//!   the conversation is configured for client-side responses.
//!
//! This crate intentionally has no platform deps: mic capture and
//! speaker playback live in `atomr-agents-stt-audio` and
//! `atomr-agents-tts-audio` respectively. Consumers wire those up
//! around `Conversation`.

mod conversation;

pub use conversation::{
    Conversation, ConversationAgent, ConversationEvent, ConversationMode, ConversationOptions,
    NoopAgent,
};
