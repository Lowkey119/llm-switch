//! Backend protocol adapters. Each adapter knows how to reach one upstream
//! wire protocol and (optionally) how to translate it to/from the entry
//! protocol the client spoke.

pub mod adapter;
pub mod anthropic;
pub mod chat_anthropic;
pub mod codex_chat_common;
pub mod codex_compaction;
pub mod codex_responses_sse;
pub mod gemini;
pub mod gemini_inbound;
pub mod gemini_schema;
pub mod gemini_shadow;
pub mod inline_think;
pub mod models;
pub mod ollama_inbound;
pub mod openai_chat;
pub mod openai_responses;
pub mod reasoning_bridge;
pub mod streaming;
pub mod streaming_codex_chat;
pub mod streaming_gemini;
pub mod streaming_responses;
pub mod transform;
pub mod transform_codex_chat;
pub mod transform_gemini;
pub mod transform_responses;

pub use adapter::adapter_for;
