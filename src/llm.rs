//! LLM client abstraction.
//!
//! The pipeline never talks to a concrete HTTP library directly — every
//! stage module calls through the `LlmClient` trait. That gives us two
//! implementations:
//!
//! - `HttpLlmClient` — the real, air-gapped-deployment client. Speaks the
//!   OpenAI-compatible `chat/completions` HTTP shape, which is what
//!   vLLM, llama.cpp's server, Ollama's OpenAI-compat endpoint, TGI, and
//!   LM Studio all implement. This is what `legacy-modernizer.toml`
//!   points at in a real engagement.
//! - `MockLlmClient` — a deterministic, offline, canned-response client
//!   used by the `demo` subcommand and by this crate's own test suite
//!   (see `tests/prototype_demo.rs`) to prove the six-stage pipeline
//!   actually runs end-to-end without requiring a real model server.
//!   This is what lets someone clone this repo and see the agents work
//!   before they've stood up any LLM at all.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Everything a pipeline stage needs from "the model": send a system
/// prompt + a user prompt, get back the model's text response.
pub trait LlmClient {
    fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;
}

// ---------------------------------------------------------------------
// Real client — OpenAI-compatible HTTP API
// ---------------------------------------------------------------------

pub struct HttpLlmClient {
    base_url: String,
    api_key: Option<String>,
    model: String,
    temperature: f32,
    client: reqwest::blocking::Client,
}

impl HttpLlmClient {
    pub fn new(base_url: String, api_key: Option<String>, model: String, temperature: f32) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .context("failed to construct HTTP client")?;
        Ok(HttpLlmClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            temperature,
            client,
        })
    }
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

impl LlmClient for HttpLlmClient {
    fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage { role: "system", content: system_prompt },
                ChatMessage { role: "user", content: user_prompt },
            ],
            temperature: self.temperature,
        };

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .with_context(|| format!("request to LLM endpoint failed: {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            bail!("LLM endpoint returned HTTP {status}: {text}");
        }

        let parsed: ChatResponse = resp
            .json()
            .context("LLM endpoint response was not the expected OpenAI-compatible chat/completions shape")?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .context("LLM endpoint returned zero choices")
    }
}

// ---------------------------------------------------------------------
// Mock client — deterministic, offline, for `demo` and the test suite
// ---------------------------------------------------------------------

/// Returns a fixed, pre-scripted sequence of responses, one per call, in
/// order. Used to drive a full six-stage pipeline run with no network
/// access and no real model — this is the "prototype" harness: it proves
/// the orchestration (sequencing, gating, file I/O, report generation)
/// works correctly, independent of any particular model's output
/// quality.
pub struct MockLlmClient {
    queue: Mutex<VecDeque<String>>,
}

impl MockLlmClient {
    pub fn new(responses: Vec<String>) -> Self {
        MockLlmClient {
            queue: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl LlmClient for MockLlmClient {
    fn chat(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String> {
        let mut q = self.queue.lock().expect("mock LLM queue poisoned");
        q.pop_front()
            .context("MockLlmClient ran out of scripted responses — the pipeline made more LLM calls than the demo/test script provided")
    }
}
