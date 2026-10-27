//! OpenRouter's public model catalog, which the setup wizard offers as a searchable list.
//!
//! OpenRouter fronts models from every vendor behind one API key; a model is addressed as
//! `openrouter:<vendor>/<model>`. The catalog endpoint needs no key.

use std::time::Duration;

use serde::Deserialize;

pub const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// Catalog entries shown first when nothing is typed, in this order.
pub const RECOMMENDED: &[&str] = &[
    "google/gemini-3.8-flash",
    "openai/gpt-6.1-terra",
    "anthropic/claude-sonnet-5.5",
    "x-ai/grok-4.7",
];

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
    /// US dollars per million prompt tokens; `None` when the catalog gives no price.
    pub prompt_per_m: Option<f64>,
    pub completion_per_m: Option<f64>,
    pub context_length: Option<u64>,
    /// Whether the model accepts a JSON schema for its reply.
    pub structured_output: bool,
}

impl OpenRouterModel {
    /// One line for lists: price per million tokens, context length, JSON support.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        match (self.prompt_per_m, self.completion_per_m) {
            (Some(0.0), Some(0.0)) => parts.push("free".to_string()),
            (Some(p), Some(c)) => parts.push(format!("{} / {} per M", price(p), price(c))),
            _ => {}
        }
        if let Some(ctx) = self.context_length {
            parts.push(format!("{} ctx", tokens(ctx)));
        }
        if !self.structured_output {
            parts.push("no JSON schema".to_string());
        }
        parts.join(" · ")
    }
}

fn price(per_m: f64) -> String {
    let text = format!("{per_m:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("${text}")
}

fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Downloads the catalog. Runs on a worker thread in the wizard, so it blocks.
pub fn fetch_models() -> Result<Vec<OpenRouterModel>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("vibegit/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(MODELS_URL)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| e.to_string())?;
    parse_catalog(&body)
}

#[derive(Deserialize)]
struct Catalog {
    data: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    pricing: Option<Pricing>,
    #[serde(default)]
    architecture: Option<Architecture>,
    #[serde(default)]
    supported_parameters: Vec<String>,
}

#[derive(Deserialize)]
struct Pricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

#[derive(Deserialize)]
struct Architecture {
    #[serde(default)]
    output_modalities: Vec<String>,
}

/// Parses the catalog JSON, keeping text models addressable as `vendor/model` and
/// dropping batch-only variants, which cannot answer an interactive request.
pub fn parse_catalog(json: &str) -> Result<Vec<OpenRouterModel>, String> {
    let catalog: Catalog =
        serde_json::from_str(json).map_err(|e| format!("unexpected catalog format: {e}"))?;
    let models = catalog
        .data
        .into_iter()
        .filter(|e| is_plain_slug(&e.id) && !e.id.ends_with(":batch"))
        .filter(|e| {
            e.architecture
                .as_ref()
                .map_or(true, |a| a.output_modalities.iter().any(|m| m == "text"))
        })
        .map(|e| {
            let per_m = |s: &Option<String>| s.as_deref()?.parse::<f64>().ok().map(|v| v * 1e6);
            let (prompt, completion) = match &e.pricing {
                Some(p) => (per_m(&p.prompt), per_m(&p.completion)),
                None => (None, None),
            };
            OpenRouterModel {
                structured_output: e
                    .supported_parameters
                    .iter()
                    .any(|p| p == "structured_outputs" || p == "response_format"),
                id: e.id,
                name: e.name,
                prompt_per_m: prompt,
                completion_per_m: completion,
                context_length: e.context_length,
            }
        })
        .collect();
    Ok(models)
}

/// `vendor/model` with an optional `:variant` suffix; excludes alias entries such as `~vendor/latest`.
fn is_plain_slug(id: &str) -> bool {
    let Some((vendor, model)) = id.split_once('/') else {
        return false;
    };
    let ok = |s: &str, extra: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c) || extra.contains(c))
    };
    ok(vendor, "") && ok(model, ":")
}

/// Whether `id` looks like something OpenRouter can route, for names typed by hand.
pub fn looks_like_model(id: &str) -> bool {
    is_plain_slug(id)
}

/// Models whose id or name contains every whitespace-separated word of `filter`, ignoring
/// case. With an empty filter the recommended models come first, then the catalog order.
pub fn filter_models<'a>(models: &'a [OpenRouterModel], filter: &str) -> Vec<&'a OpenRouterModel> {
    let words: Vec<String> = filter
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let matches = |m: &OpenRouterModel| {
        let id = m.id.to_ascii_lowercase();
        let name = m.name.to_ascii_lowercase();
        words.iter().all(|w| id.contains(w) || name.contains(w))
    };
    let mut out: Vec<&OpenRouterModel> = Vec::new();
    if words.is_empty() {
        out.extend(
            RECOMMENDED
                .iter()
                .filter_map(|id| models.iter().find(|m| m.id == *id)),
        );
    }
    let pinned: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
    out.extend(
        models
            .iter()
            .filter(|m| matches(m) && !pinned.contains(&m.id.as_str())),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"data":[
      {"id":"openai/gpt-6.1-terra","name":"OpenAI: GPT-6.1 Terra","context_length":1050000,
       "pricing":{"prompt":"0.000001","completion":"0.000005"},
       "architecture":{"output_modalities":["text"]},
       "supported_parameters":["response_format","structured_outputs","tools"]},
      {"id":"openai/gpt-6.1-terra:batch","name":"OpenAI: GPT-6.1 Terra (batch)",
       "architecture":{"output_modalities":["text"]},"supported_parameters":[]},
      {"id":"google/gemini-3.8-flash","name":"Google: Gemini 3.8 Flash","context_length":1048576,
       "pricing":{"prompt":"0.00000075","completion":"0.00000375"},
       "architecture":{"output_modalities":["text"]},"supported_parameters":["structured_outputs"]},
      {"id":"someone/painter","name":"Painter","architecture":{"output_modalities":["image"]}},
      {"id":"~vendor/latest","name":"Alias","architecture":{"output_modalities":["text"]}},
      {"id":"meta/llama-free:free","name":"Llama (free)","context_length":8192,
       "pricing":{"prompt":"0","completion":"0"},
       "architecture":{"output_modalities":["text"]},"supported_parameters":["max_tokens"]}
    ]}"#;

    #[test]
    fn parses_text_models_and_skips_batch_image_and_alias_entries() {
        let models = parse_catalog(SAMPLE).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "openai/gpt-6.1-terra",
                "google/gemini-3.8-flash",
                "meta/llama-free:free"
            ]
        );
        let terra = &models[0];
        assert_eq!(terra.prompt_per_m, Some(1.0));
        assert_eq!(terra.completion_per_m, Some(5.0));
        assert!(terra.structured_output);
        assert_eq!(terra.summary(), "$1 / $5 per M · 1.1M ctx");
        assert_eq!(models[1].summary(), "$0.75 / $3.75 per M · 1.0M ctx");
        assert_eq!(models[2].summary(), "free · 8K ctx · no JSON schema");
        assert!(parse_catalog("[]").is_err());
    }

    #[test]
    fn filters_by_every_word_and_recommends_first() {
        let models = parse_catalog(SAMPLE).unwrap();
        let all: Vec<&str> = filter_models(&models, "")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            all,
            [
                "google/gemini-3.8-flash",
                "openai/gpt-6.1-terra",
                "meta/llama-free:free"
            ]
        );
        let hits: Vec<&str> = filter_models(&models, "GPT terra")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(hits, ["openai/gpt-6.1-terra"]);
        assert!(filter_models(&models, "nothing-like-this").is_empty());
    }

    #[test]
    fn hand_typed_names_need_a_vendor_prefix() {
        assert!(looks_like_model("google/gemini-3.8-flash"));
        assert!(looks_like_model("meta/llama:free"));
        assert!(!looks_like_model("gemini-3.8-flash"));
        assert!(!looks_like_model("vendor/model name"));
        assert!(!looks_like_model("/model"));
    }
}
