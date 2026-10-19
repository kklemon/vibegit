//! The model's output: commit proposals and excluded groups, plus the JSON
//! schemas sent to providers that support structured output.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitProposal {
    #[serde(alias = "reasoning", default)]
    pub explanation: String,
    #[serde(default)]
    pub commit_message: String,
    #[serde(deserialize_with = "deserialize_ids", default)]
    pub change_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExcludedGroup {
    #[serde(alias = "reasoning", default)]
    pub explanation: String,
    #[serde(deserialize_with = "deserialize_ids", default)]
    pub change_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProposalResult {
    #[serde(default)]
    pub commit_proposals: Vec<CommitProposal>,
    #[serde(default)]
    pub excluded_groups: Vec<ExcludedGroup>,
}

/// Accepts integers or numeric strings; models occasionally quote the ids.
fn deserialize_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Num(u64),
        Str(String),
    }
    let raw: Vec<Id> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|id| match id {
            Id::Num(n) => u32::try_from(n).map_err(serde::de::Error::custom),
            Id::Str(s) => s
                .trim()
                .parse::<u32>()
                .map_err(|_| serde::de::Error::custom(format!("invalid change id {s:?}"))),
        })
        .collect()
}

/// JSON schema of the expected response.
///
/// `strict` adds `additionalProperties: false` and lists every property as required,
/// which OpenAI's strict mode demands and Gemini's schema dialect rejects.
pub fn output_schema(allow_excluding: bool, strict: bool) -> Value {
    let ids = json!({
        "type": "array",
        "description": "Change IDs (integers) that belong to this group.",
        "items": {"type": "integer"}
    });
    let mut proposal = json!({
        "type": "object",
        "properties": {
            "explanation": {"type": "string", "description": "Why these changes belong together."},
            "commit_message": {"type": "string", "description": "The proposed commit message."},
            "change_ids": ids
        },
        "required": ["explanation", "commit_message", "change_ids"]
    });
    let mut excluded = json!({
        "type": "object",
        "properties": {
            "explanation": {"type": "string", "description": "Why these changes are not ready to be committed."},
            "change_ids": ids
        },
        "required": ["explanation", "change_ids"]
    });
    let mut root = json!({
        "type": "object",
        "properties": {
            "commit_proposals": {
                "type": "array",
                "description": "Proposed commits, in the order they should be applied.",
                "items": proposal.clone()
            }
        },
        "required": ["commit_proposals"]
    });
    if allow_excluding {
        root["properties"]["excluded_groups"] = json!({
            "type": "array",
            "description": "Groups of changes that should not be committed yet.",
            "items": excluded.clone()
        });
        root["required"] = json!(["commit_proposals", "excluded_groups"]);
    }
    if strict {
        for obj in [&mut proposal, &mut excluded, &mut root] {
            obj["additionalProperties"] = json!(false);
        }
        root["properties"]["commit_proposals"]["items"] = proposal;
        if allow_excluding {
            root["properties"]["excluded_groups"]["items"] = excluded;
        }
    }
    root
}

/// Extracts the JSON object from a model reply, tolerating code fences and prose around it.
pub fn parse_response(text: &str) -> Result<ProposalResult, String> {
    let trimmed = text.trim();
    let candidate = if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        v
    } else {
        let start = trimmed.find('{').ok_or("no JSON object in the response")?;
        let end = trimmed.rfind('}').ok_or("no JSON object in the response")?;
        if end <= start {
            return Err("no JSON object in the response".into());
        }
        serde_json::from_str::<Value>(&trimmed[start..=end])
            .map_err(|e| format!("invalid JSON: {e}"))?
    };
    serde_json::from_value::<ProposalResult>(candidate)
        .map_err(|e| format!("unexpected response shape: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_fenced_json_with_string_ids() {
        let plain = r#"{"commit_proposals":[{"explanation":"x","commit_message":"m","change_ids":[1,"2"]}]}"#;
        let r = parse_response(plain).unwrap();
        assert_eq!(r.commit_proposals[0].change_ids, vec![1, 2]);
        let fenced = "Here you go:\n```json\n{\"commit_proposals\":[{\"reasoning\":\"x\",\"commit_message\":\"m\",\"change_ids\":[3]}],\"excluded_groups\":[{\"explanation\":\"wip\",\"change_ids\":[4]}]}\n```";
        let r = parse_response(fenced).unwrap();
        assert_eq!(r.commit_proposals[0].explanation, "x");
        assert_eq!(r.excluded_groups[0].change_ids, vec![4]);
        assert!(parse_response("nothing here").is_err());
    }

    #[test]
    fn schema_variants() {
        let s = output_schema(true, true);
        assert_eq!(s["additionalProperties"], json!(false));
        assert!(s["properties"]["excluded_groups"].is_object());
        let g = output_schema(false, false);
        assert!(g.get("additionalProperties").is_none());
        assert!(g["properties"].get("excluded_groups").is_none());
    }
}
