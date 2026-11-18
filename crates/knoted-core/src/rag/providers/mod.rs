//! Explicit model selection and OpenAI-compatible answer streaming.
//!
//! The worker owns a provider stream for one generation. Only answer deltas
//! leave this module, and both the terminal reason and the SSE done marker are
//! required for completion. The caller owns persistence and resource leases.

mod config;
mod sse;

pub use config::{ChatProvidersConfig, ProviderConfig};

use std::{fmt, fs::File, future::Future, io::Read, time::Duration};

use reqwest::{
    Client, Method, RequestBuilder, Response, Url,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::watch;

pub const CONTEXT_WINDOW: usize = 32 * 1024;
pub const OUTPUT_RESERVED_TOKENS: usize = 4 * 1024;
const STATUS_LIMIT: usize = 64 * 1024;
const CREDENTIAL_LIMIT: u64 = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("model provider configuration is invalid")]
    InvalidConfig,
    #[error("selected model provider is not configured")]
    NotConfigured,
    #[error("model provider credential is unavailable")]
    CredentialUnavailable,
    #[error("model provider is unavailable")]
    Unavailable,
    #[error("model provider rejected the request")]
    UpstreamRejected,
    #[error("model provider sent a stream error")]
    UpstreamStreamError,
    #[error("model provider stream was interrupted")]
    Interrupted,
    #[error("model provider response does not match the protocol")]
    InvalidProtocol,
    #[error("model provider returned a different model")]
    ModelMismatch,
    #[error("chat input exceeds the model context budget")]
    ContextTooLarge,
    #[error("chat request is invalid")]
    InvalidRequest,
    #[error("model generation was cancelled")]
    Cancelled { confirmed: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    pub name: String,
    pub model: String,
    pub context_window: usize,
    pub max_input_tokens: usize,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Serialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    /// Count from the caller's verified context budget. With no model-native
    /// tokenizer, compute a conservative count before passing the request.
    pub input_tokens: usize,
    pub max_output_tokens: usize,
}

impl fmt::Debug for ChatRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatRequest")
            .field("message_count", &self.messages.len())
            .field("input_tokens", &self.input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
}

#[derive(PartialEq, Eq)]
pub enum ProviderEvent {
    Delta(String),
    Finished(FinishReason),
}

impl fmt::Debug for ProviderEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Delta(text) => f.debug_tuple("Delta").field(&text.len()).finish(),
            Self::Finished(reason) => f.debug_tuple("Finished").field(reason).finish(),
        }
    }
}

/// A cancellation signal shared by one generation and its lease owner.
#[derive(Clone)]
pub struct ChatCancellation {
    signal: watch::Sender<bool>,
}

impl Default for ChatCancellation {
    fn default() -> Self {
        Self {
            signal: watch::channel(false).0,
        }
    }
}

impl ChatCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.signal.send_replace(true);
    }

    async fn cancelled(&self) {
        let mut receiver = self.signal.subscribe();
        let _ = receiver.wait_for(|flag| *flag).await;
    }
}

pub trait ChatProvider {
    fn info(&self) -> impl Future<Output = Result<ProviderInfo, ProviderError>> + Send;

    fn stream(
        &self,
        request: ChatRequest,
        cancel: ChatCancellation,
    ) -> impl Future<Output = Result<ChatStream, ProviderError>> + Send;

    /// Observes whether a disconnected upstream generation has stopped.
    ///
    /// For a generic external provider a closed HTTP stream has no confirmed
    /// stop semantics. The caller must keep that resource state unresolved.
    fn wait_for_stop(&self) -> impl Future<Output = bool> + Send;
}

#[derive(Debug)]
pub struct ProviderRegistry {
    config: ChatProvidersConfig,
    http: Client,
}

impl ProviderRegistry {
    pub fn new(config: ChatProvidersConfig) -> Result<Self, ProviderError> {
        config.validate()?;
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .build()
            .map_err(|_| ProviderError::Unavailable)?;
        Ok(Self { config, http })
    }

    pub fn select(&self, name: Option<&str>) -> Result<HttpChatProvider, ProviderError> {
        let name = name.unwrap_or("local");
        let local = name == "local";
        let spec = if local {
            self.config.local.as_ref()
        } else {
            self.config.external.get(name)
        }
        .ok_or(ProviderError::NotConfigured)?;
        let base = spec.api_url(local)?;
        let auth = credential(spec)?;
        Ok(HttpChatProvider {
            name: name.to_owned(),
            model: spec.model.clone(),
            local,
            base,
            auth,
            http: self.http.clone(),
        })
    }
}

fn credential(spec: &ProviderConfig) -> Result<HeaderValue, ProviderError> {
    let mut bytes = String::new();
    let file =
        File::open(&spec.credential_file).map_err(|_| ProviderError::CredentialUnavailable)?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(ProviderError::CredentialUnavailable);
    }
    file.take(CREDENTIAL_LIMIT + 1)
        .read_to_string(&mut bytes)
        .map_err(|_| ProviderError::CredentialUnavailable)?;
    let secret = bytes.trim();
    if bytes.len() as u64 > CREDENTIAL_LIMIT
        || secret.is_empty()
        || !secret.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(ProviderError::CredentialUnavailable);
    }
    let mut header = HeaderValue::from_str(&format!("Bearer {secret}"))
        .map_err(|_| ProviderError::CredentialUnavailable)?;
    header.set_sensitive(true);
    Ok(header)
}

#[derive(Clone)]
pub struct HttpChatProvider {
    name: String,
    model: String,
    local: bool,
    base: Url,
    auth: HeaderValue,
    http: Client,
}

impl fmt::Debug for HttpChatProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpChatProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct StrataStatus {
    service: String,
    model: String,
    loaded: bool,
    cache_max_tokens: usize,
    context: StrataContext,
    activity: StrataActivity,
}

#[derive(Deserialize)]
struct StrataContext {
    native: usize,
    max_positions: usize,
}

#[derive(Deserialize)]
struct StrataActivity {
    in_flight: usize,
}

#[derive(Deserialize)]
struct StrataModels {
    data: Vec<StrataModel>,
}

#[derive(Deserialize)]
struct StrataModel {
    id: String,
    meta: ModelMeta,
}

#[derive(Deserialize)]
struct ModelMeta {
    n_ctx: usize,
}

impl HttpChatProvider {
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, self.base.join(path).expect("fixed model endpoint"))
            .header(AUTHORIZATION, self.auth.clone())
    }

    async fn local_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ProviderError> {
        let mut response = self
            .request(Method::GET, path)
            .send()
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        check_status(&response)?;
        let mut body = Vec::new();
        while let Some(part) = response
            .chunk()
            .await
            .map_err(|_| ProviderError::Unavailable)?
        {
            if body.len() + part.len() > STATUS_LIMIT {
                return Err(ProviderError::InvalidProtocol);
            }
            body.extend_from_slice(&part);
        }
        serde_json::from_slice(&body).map_err(|_| ProviderError::InvalidProtocol)
    }

    async fn strata_status(&self) -> Result<StrataStatus, ProviderError> {
        let status: StrataStatus = self.local_json("status").await?;
        if status.service != "strata" || !status.loaded {
            return Err(ProviderError::Unavailable);
        }
        if status.model != self.model {
            return Err(ProviderError::ModelMismatch);
        }
        Ok(status)
    }
}

fn check_status(response: &Response) -> Result<(), ProviderError> {
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Err(ProviderError::Unavailable)
    } else {
        Err(ProviderError::UpstreamRejected)
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    max_tokens: usize,
    stream: bool,
}

impl ChatProvider for HttpChatProvider {
    async fn info(&self) -> Result<ProviderInfo, ProviderError> {
        let mut context_window = CONTEXT_WINDOW;
        if self.local {
            let (status, models) = tokio::try_join!(
                self.strata_status(),
                self.local_json::<StrataModels>("models")
            )?;
            let model = models
                .data
                .iter()
                .find(|model| model.id == self.model)
                .ok_or(ProviderError::ModelMismatch)?;
            context_window = context_window
                .min(status.context.native)
                .min(status.context.max_positions)
                .min(status.cache_max_tokens)
                .min(model.meta.n_ctx);
        }
        let max_input_tokens = context_window
            .checked_sub(OUTPUT_RESERVED_TOKENS)
            .filter(|value| *value > 0)
            .ok_or(ProviderError::ContextTooLarge)?;
        Ok(ProviderInfo {
            name: self.name.clone(),
            model: self.model.clone(),
            context_window,
            max_input_tokens,
        })
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: ChatCancellation,
    ) -> Result<ChatStream, ProviderError> {
        if request.messages.is_empty()
            || request.input_tokens == 0
            || !(1..=OUTPUT_RESERVED_TOKENS).contains(&request.max_output_tokens)
        {
            return Err(ProviderError::InvalidRequest);
        }
        let info = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProviderError::Cancelled { confirmed: true }),
            info = self.info() => info?,
        };
        if request.input_tokens > info.max_input_tokens {
            return Err(ProviderError::ContextTooLarge);
        }

        let upstream = self
            .request(Method::POST, "chat/completions")
            .header(ACCEPT, "text/event-stream")
            .json(&WireRequest {
                model: &self.model,
                messages: &request.messages,
                max_tokens: request.max_output_tokens,
                stream: true,
            });
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            response = upstream.send() => Some(response),
        };
        let Some(response) = response else {
            return Err(ProviderError::Cancelled {
                confirmed: self.wait_for_stop().await,
            });
        };
        let response = response.map_err(|_| ProviderError::Unavailable)?;
        check_status(&response)?;
        let is_sse = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.split(';').next())
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"));
        if !is_sse {
            return Err(ProviderError::InvalidProtocol);
        }
        Ok(ChatStream {
            response: Some(response),
            provider: self.clone(),
            cancel,
            decoder: sse::Decoder::default(),
            finish_reason: None,
        })
    }

    async fn wait_for_stop(&self) -> bool {
        if !self.local {
            return false;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self
                    .strata_status()
                    .await
                    .is_ok_and(|status| status.activity.in_flight == 0)
                {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or(false)
    }
}

/// Owns the upstream HTTP stream until terminal proof, operator cancellation
/// or interruption. Dropping it closes the transport.
pub struct ChatStream {
    response: Option<Response>,
    provider: HttpChatProvider,
    cancel: ChatCancellation,
    decoder: sse::Decoder,
    finish_reason: Option<FinishReason>,
}

impl fmt::Debug for ChatStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatStream")
            .field("provider", &self.provider)
            .field("open", &self.response.is_some())
            .finish()
    }
}

#[derive(Deserialize)]
struct WireChunk {
    model: Option<String>,
    #[serde(default)]
    choices: Vec<WireChoice>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct WireChoice {
    #[serde(default)]
    index: usize,
    delta: Option<WireDelta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireDelta {
    content: Option<String>,
}

impl ChatStream {
    pub async fn next(&mut self) -> Option<Result<ProviderEvent, ProviderError>> {
        self.response.as_ref()?;
        let result = self.next_event().await;
        if !matches!(result, Ok(ProviderEvent::Delta(_))) {
            self.response = None;
        }
        Some(result)
    }

    async fn next_event(&mut self) -> Result<ProviderEvent, ProviderError> {
        loop {
            // Check the cancellation before draining already-received bytes.
            // The caller must not persist deltas after requesting cancellation.
            if *self.cancel.signal.borrow() {
                return self.cancelled().await;
            }
            if let Some(frame) = self.decoder.next()? {
                if let Some(event) = self.event(frame)? {
                    return Ok(event);
                }
                continue;
            }
            let response = self.response.as_mut().expect("the response is open");
            let chunk = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => None,
                chunk = response.chunk() => Some(chunk),
            };
            match chunk {
                None => return self.cancelled().await,
                Some(Ok(Some(bytes))) => self.decoder.push(&bytes),
                Some(Ok(None) | Err(_)) => return Err(ProviderError::Interrupted),
            }
        }
    }

    async fn cancelled(&mut self) -> Result<ProviderEvent, ProviderError> {
        self.response = None;
        Err(ProviderError::Cancelled {
            confirmed: self.provider.wait_for_stop().await,
        })
    }

    fn event(&mut self, frame: sse::Frame) -> Result<Option<ProviderEvent>, ProviderError> {
        if frame.is_error {
            return Err(ProviderError::UpstreamStreamError);
        }
        if frame.data == b"[DONE]" {
            let reason = self.finish_reason.ok_or(ProviderError::InvalidProtocol)?;
            return Ok(Some(ProviderEvent::Finished(reason)));
        }
        let mut chunk: WireChunk =
            serde_json::from_slice(&frame.data).map_err(|_| ProviderError::InvalidProtocol)?;
        if chunk.error.is_some() {
            return Err(ProviderError::UpstreamStreamError);
        }
        if chunk.choices.is_empty() {
            return Ok(None); // usage-only record
        }
        if self.provider.local && chunk.model.as_deref() != Some(self.provider.model.as_str()) {
            return Err(ProviderError::ModelMismatch);
        }
        if chunk.choices.len() != 1 || self.finish_reason.is_some() {
            return Err(ProviderError::InvalidProtocol);
        }
        let choice = chunk.choices.pop().expect("one choice");
        if choice.index != 0 {
            return Err(ProviderError::InvalidProtocol);
        }
        self.finish_reason = match choice.finish_reason.as_deref() {
            None => None,
            Some("stop") => Some(FinishReason::Stop),
            Some("length") => Some(FinishReason::Length),
            Some(_) => return Err(ProviderError::UpstreamStreamError),
        };
        Ok(choice
            .delta
            .and_then(|delta| delta.content)
            .filter(|value| !value.is_empty())
            .map(ProviderEvent::Delta))
    }
}
