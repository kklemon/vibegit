//! Model access through the `genai` crate, which speaks to Gemini, OpenAI, xAI,
//! Anthropic, Ollama and other providers, including any OpenAI-compatible endpoint.

use std::time::Duration;

use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat, JsonSpec};
use genai::resolver::{AuthData, Endpoint};
use genai::{Client, ModelIden, ServiceTarget};

use crate::config::Config;
use crate::proposal::{output_schema, parse_response, ProposalResult};

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("{0}")]
    Config(String),
    #[error("model request failed: {0}")]
    Request(String),
    #[error("the model returned an empty response")]
    Empty,
    #[error("the model's response was invalid after {attempts} attempt(s): {reason}\n\nLast response:\n{raw}")]
    Invalid {
        attempts: usize,
        reason: String,
        raw: String,
    },
    #[error("could not start the async runtime: {0}")]
    Runtime(String),
}

/// What the workflow hands to a proposer.
pub struct ProposeRequest<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub allow_excluding: bool,
    /// Semantic validation against the change set; a failure triggers one retry.
    pub validate: &'a dyn Fn(&ProposalResult) -> Result<(), String>,
}

pub trait Proposer: Send {
    fn propose(&self, req: ProposeRequest<'_>) -> Result<ProposalResult, LlmError>;
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub adapter: AdapterKind,
    pub provider: String,
    pub model: String,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub temperature: Option<f64>,
}

impl ResolvedModel {
    pub fn label(&self) -> String {
        if self.endpoint.is_some() {
            format!(
                "{} @ {}",
                self.model,
                self.endpoint.as_deref().unwrap_or("")
            )
        } else {
            format!("{}:{}", self.provider, self.model)
        }
    }
}

/// Maps a provider name (including the legacy pydantic-ai spellings) to a genai adapter.
pub fn provider_adapter(provider: &str) -> Option<(AdapterKind, &'static str)> {
    let kind = match provider.to_ascii_lowercase().as_str() {
        "google" | "google-gla" | "google_genai" | "google-genai" | "gemini" => {
            (AdapterKind::Gemini, "google")
        }
        "openai" => (AdapterKind::OpenAI, "openai"),
        "grok" | "xai" => (AdapterKind::Xai, "grok"),
        "anthropic" | "claude" => (AdapterKind::Anthropic, "anthropic"),
        "ollama" => (AdapterKind::Ollama, "ollama"),
        "groq" => (AdapterKind::Groq, "groq"),
        "deepseek" => (AdapterKind::DeepSeek, "deepseek"),
        "cohere" => (AdapterKind::Cohere, "cohere"),
        "openrouter" => (AdapterKind::OpenRouter, "openrouter"),
        "together" => (AdapterKind::Together, "together"),
        "fireworks" => (AdapterKind::Fireworks, "fireworks"),
        _ => return None,
    };
    Some(kind)
}

pub const SUPPORTED_PROVIDERS: &[&str] = &[
    "google",
    "openai",
    "grok",
    "anthropic",
    "ollama",
    "groq",
    "deepseek",
    "cohere",
    "openrouter",
    "together",
    "fireworks",
];

/// Environment variable names (and `api_keys` entries) that hold the key for an adapter.
pub fn api_key_names(adapter: AdapterKind) -> &'static [&'static str] {
    match adapter {
        AdapterKind::Gemini => &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
        AdapterKind::OpenAI => &["OPENAI_API_KEY"],
        AdapterKind::Xai => &["GROK_API_KEY", "XAI_API_KEY"],
        AdapterKind::Anthropic => &["ANTHROPIC_API_KEY"],
        AdapterKind::Groq => &["GROQ_API_KEY"],
        AdapterKind::DeepSeek => &["DEEPSEEK_API_KEY"],
        AdapterKind::Cohere => &["COHERE_API_KEY"],
        AdapterKind::OpenRouter => &["OPENROUTER_API_KEY"],
        AdapterKind::Together => &["TOGETHER_API_KEY"],
        AdapterKind::Fireworks => &["FIREWORKS_API_KEY"],
        _ => &[],
    }
}

fn lookup_key(config: &Config, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(v) = config.api_keys.get(*name).filter(|v| !v.trim().is_empty()) {
            return Some(v.trim().to_string());
        }
    }
    for name in names {
        if let Ok(v) = std::env::var(name) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Splits `provider:model`, falling back to `model_provider` for bare names.
pub fn split_model_name(name: &str, model_provider: Option<&str>) -> (Option<String>, String) {
    if let Some((p, m)) = name.split_once(':') {
        if !p.is_empty() && !m.is_empty() {
            return (Some(p.to_string()), m.to_string());
        }
    }
    (model_provider.map(|p| p.to_string()), name.to_string())
}

/// Resolves the configured model to an adapter, endpoint and credentials.
pub fn resolve_model(config: &Config) -> Result<ResolvedModel, LlmError> {
    let mc = &config.model;
    let temperature = mc.temperature;

    if let Some(base_url) = mc
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        // Custom OpenAI-compatible endpoint.
        let (_, model) = split_model_name(&mc.name, None);
        let mut endpoint = base_url.to_string();
        if !endpoint.ends_with('/') {
            endpoint.push('/');
        }
        let api_key = mc
            .api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| lookup_key(config, api_key_names(AdapterKind::OpenAI)))
            .unwrap_or_else(|| "no-key".to_string());
        return Ok(ResolvedModel {
            adapter: AdapterKind::OpenAI,
            provider: "custom".into(),
            model,
            endpoint: Some(endpoint),
            api_key: Some(api_key),
            temperature,
        });
    }

    let (provider, model) = split_model_name(&mc.name, mc.model_provider.as_deref());
    let provider = provider.ok_or_else(|| {
        LlmError::Config(format!(
            "model name {:?} must use the <provider>:<model> format, e.g. google:gemini-3.8-flash",
            mc.name
        ))
    })?;
    let (adapter, provider_label) = provider_adapter(&provider).ok_or_else(|| {
        LlmError::Config(format!(
            "unknown model provider {provider:?}; supported providers: {}. For other OpenAI-compatible APIs set model.base_url.",
            SUPPORTED_PROVIDERS.join(", ")
        ))
    })?;
    let names = api_key_names(adapter);
    let api_key = match mc.api_key.clone().filter(|k| !k.trim().is_empty()) {
        Some(k) => Some(k),
        None if names.is_empty() => Some("ollama".to_string()),
        None => match lookup_key(config, names) {
            Some(k) => Some(k),
            None => {
                return Err(LlmError::Config(format!(
                    "no API key found for {provider_label}. Run `vibegit config` or `vibegit config set api_keys.{} <key>`, or export {} in your shell.",
                    names[0], names[0]
                )))
            }
        },
    };
    Ok(ResolvedModel {
        adapter,
        provider: provider_label.to_string(),
        model,
        endpoint: None,
        api_key,
        temperature,
    })
}

/// Deterministic proposer for tests and interface smoke runs: every change unit
/// becomes its own commit. Enabled with the `VIBEGIT_FAKE_PROPOSER` environment variable.
pub struct FakeProposer;

impl Proposer for FakeProposer {
    fn propose(&self, req: ProposeRequest<'_>) -> Result<ProposalResult, LlmError> {
        let ids: Vec<u32> = req
            .user
            .lines()
            .filter_map(|l| l.strip_prefix("# Change ID: "))
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let result = ProposalResult {
            commit_proposals: ids
                .iter()
                .map(|id| crate::proposal::CommitProposal {
                    explanation: format!("Fake proposal for change {id}."),
                    commit_message: format!("fake commit for change {id}"),
                    change_ids: vec![*id],
                })
                .collect(),
            excluded_groups: Vec::new(),
        };
        (req.validate)(&result).map_err(|reason| LlmError::Invalid {
            attempts: 1,
            reason,
            raw: String::new(),
        })?;
        Ok(result)
    }
}

/// A proposer that calls the configured model, retrying once with the validation error.
pub struct LlmProposer {
    pub model: ResolvedModel,
    pub timeout: Duration,
}

impl LlmProposer {
    pub fn new(model: ResolvedModel) -> Self {
        Self {
            model,
            timeout: Duration::from_secs(240),
        }
    }

    fn client(&self) -> Result<Client, LlmError> {
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| LlmError::Request(e.to_string()))?;
        let mut builder = Client::builder().with_reqwest(http);
        if let Some(key) = self.model.api_key.clone() {
            builder = builder.with_auth_resolver_fn(
                move |_model: ModelIden| -> genai::resolver::Result<Option<AuthData>> {
                    Ok(Some(AuthData::from_single(key.clone())))
                },
            );
        }
        if let Some(url) = self.model.endpoint.clone() {
            builder = builder.with_service_target_resolver_fn(
                move |mut target: ServiceTarget| -> genai::resolver::Result<ServiceTarget> {
                    target.endpoint = Endpoint::from_owned(url.clone());
                    Ok(target)
                },
            );
        }
        Ok(builder.build())
    }

    fn options(&self, format: Option<ChatResponseFormat>) -> ChatOptions {
        let mut options = ChatOptions::default();
        if let Some(t) = self.model.temperature {
            options = options.with_temperature(t);
        }
        if let Some(f) = format {
            options = options.with_response_format(f);
        }
        options
    }

    fn formats(&self, allow_excluding: bool) -> Vec<Option<ChatResponseFormat>> {
        let spec = ChatResponseFormat::JsonSpec(JsonSpec::new(
            "commit_proposals",
            output_schema(allow_excluding, false),
        ));
        if self.model.endpoint.is_some() {
            // Unknown servers may not implement json_schema; degrade gracefully.
            vec![Some(spec), Some(ChatResponseFormat::JsonMode), None]
        } else {
            vec![Some(spec)]
        }
    }
}

impl Proposer for LlmProposer {
    fn propose(&self, req: ProposeRequest<'_>) -> Result<ProposalResult, LlmError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| LlmError::Runtime(e.to_string()))?;
        let client = self.client()?;
        let iden = ModelIden::new(self.model.adapter, self.model.model.as_str());
        let formats = self.formats(req.allow_excluding);
        let mut messages = vec![ChatMessage::system(req.system), ChatMessage::user(req.user)];
        let max_attempts = 2;
        let mut format_index = 0;
        let mut attempt = 0;
        loop {
            let options = self.options(formats[format_index].clone());
            let request = ChatRequest::new(messages.clone());
            let response =
                runtime.block_on(client.exec_chat(iden.clone(), request, Some(&options)));
            let text = match response {
                Ok(r) => r.first_text().map(|t| t.to_string()),
                Err(e) if format_index + 1 < formats.len() => {
                    // Try a less demanding response format on this server.
                    let _ = e;
                    format_index += 1;
                    continue;
                }
                Err(e) => return Err(LlmError::Request(e.to_string())),
            };
            let text = text.ok_or(LlmError::Empty)?;
            attempt += 1;
            let parsed = parse_response(&text).and_then(|r| (req.validate)(&r).map(|_| r));
            match parsed {
                Ok(result) => return Ok(result),
                Err(reason) if attempt < max_attempts => {
                    messages.push(ChatMessage::assistant(text));
                    messages.push(ChatMessage::user(format!(
                        "Your previous response was invalid: {reason}\n\nRespond again with a corrected JSON object that follows the required structure and covers every Change ID exactly once."
                    )));
                }
                Err(reason) => {
                    return Err(LlmError::Invalid {
                        attempts: attempt,
                        reason,
                        raw: text,
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelConfig;

    #[test]
    fn resolves_known_and_legacy_providers() {
        let mut config = Config::default();
        config.api_keys.insert("GOOGLE_API_KEY".into(), "g".into());
        config.model.name = "google_genai:gemini-3.8-flash".into();
        let r = resolve_model(&config).unwrap();
        assert_eq!(r.adapter, AdapterKind::Gemini);
        assert_eq!(r.label(), "google:gemini-3.8-flash");
        assert_eq!(r.api_key.as_deref(), Some("g"));

        config.model.name = "xai:grok-code-fast-1".into();
        config.api_keys.insert("XAI_API_KEY".into(), "x".into());
        let r = resolve_model(&config).unwrap();
        assert_eq!(r.adapter, AdapterKind::Xai);
        assert_eq!(r.api_key.as_deref(), Some("x"));

        config.model.name = "openai:gpt-5.6-terra".into();
        let err = resolve_model(&config).unwrap_err().to_string();
        assert!(err.contains("OPENAI_API_KEY"));

        config.model.name = "bogus:thing".into();
        assert!(resolve_model(&config)
            .unwrap_err()
            .to_string()
            .contains("unknown model provider"));
    }

    #[test]
    fn resolves_custom_endpoint() {
        let config = Config {
            model: ModelConfig {
                name: "my-model".into(),
                base_url: Some("https://api.example.com/v1".into()),
                api_key: Some("k".into()),
                model_provider: Some("openai".into()),
                temperature: Some(0.1),
            },
            ..Config::default()
        };
        let r = resolve_model(&config).unwrap();
        assert_eq!(r.adapter, AdapterKind::OpenAI);
        assert_eq!(r.endpoint.as_deref(), Some("https://api.example.com/v1/"));
        assert_eq!(r.model, "my-model");
        assert_eq!(r.temperature, Some(0.1));
    }
}
