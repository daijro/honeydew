//! Small protocol adapters: OpenAI Chat Completions, Anthropic Messages, Gemini generateContent.
use super::{Answer, INSTRUCTIONS, Message, Profile, Provider};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

pub const MAX_AUDIO: usize = 10 * 1024 * 1024;
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
#[derive(Clone)]
pub struct Audio {
    pub name: String,
    pub format: &'static str,
    pub bytes: Vec<u8>,
}
impl Audio {
    pub fn read(path: &Path) -> Result<Self, String> {
        let format = match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "mp3" => "mp3",
            "wav" => "wav",
            _ => return Err("Choose a WAV or MP3 reference.".into()),
        };
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "Could not open the audio file.")?
            .take(MAX_AUDIO as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read the audio file.")?;
        if bytes.is_empty() || bytes.len() > MAX_AUDIO {
            return Err(
                "Reference audio must be between 1 byte and 10 MB. Trim a short loop first.".into(),
            );
        }
        let valid = if format == "wav" {
            bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE")
        } else {
            bytes.starts_with(b"ID3")
                || bytes
                    .windows(2)
                    .take(4096)
                    .any(|b| b[0] == 0xff && b[1] & 0xe0 == 0xe0)
        };
        if !valid {
            return Err("The file does not appear to be WAV or MP3 audio.".into());
        }
        Ok(Self {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            format,
            bytes,
        })
    }
}

pub(super) fn base(profile: &Profile) -> Result<url::Url, String> {
    let mut url =
        url::Url::parse(profile.base_url.trim()).map_err(|_| "Enter a valid API base URL.")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use an HTTPS base URL without credentials, query parameters or a fragment.".into(),
        );
    }
    url.set_path(&format!("{}/", url.path().trim_end_matches('/')));
    Ok(url)
}
fn endpoint(profile: &Profile, path: &str) -> Result<String, String> {
    Ok(base(profile)?
        .join(path)
        .map_err(|_| "Invalid API endpoint.")?
        .into())
}
fn headers(profile: &Profile) -> Vec<(&'static str, String)> {
    match profile.provider {
        Provider::Anthropic => vec![
            ("x-api-key", profile.api_key.clone()),
            ("anthropic-version", "2023-06-01".into()),
        ],
        Provider::Gemini => vec![("x-goog-api-key", profile.api_key.clone())],
        Provider::Azure => vec![("api-key", profile.api_key.clone())],
        _ => vec![("Authorization", format!("Bearer {}", profile.api_key))],
    }
}
fn request(profile: &Profile, url: &str, body: Option<Value>) -> Result<Value, String> {
    if profile.api_key.trim().is_empty() {
        return Err("Enter your provider API key first.".into());
    }
    let bound = &profile.key_state.endpoint;
    if !bound.is_empty() && bound != base(profile)?.as_str() {
        return Err("The API URL changed. Enter or load a key for the new endpoint first.".into());
    }
    // Never forward credentials to a redirected host. Avoid logging request bodies or keys.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = if let Some(body)=body {
        let mut req=agent.post(url); for (key,value) in headers(profile) { req=req.header(key,value); }
        req.send_json(body)
    } else {
        let mut req=agent.get(url); for (key,value) in headers(profile) { req=req.header(key,value); }
        req.call()
    }.map_err(|_|"Could not reach the provider. Check the URL, connection and TLS certificate; the request may have timed out.".to_owned())?;
    let status = response.status().as_u16();
    if status == 429 {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let error = response
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_json::<Value>()
            .unwrap_or(Value::Null);
        return Err(rate_limit_message(&error, retry_after));
    }
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => format!(
                "Provider returned HTTP {status}. Check your API key and model permissions."
            ),
            404 => "Provider returned HTTP 404. Check the API base URL and model/deployment name."
                .into(),
            400 | 422 => format!(
                "Provider returned HTTP {status}. The selected model may not support this chat/audio format; try another model or turn audio input off."
            ),
            _ => format!("Provider returned HTTP {status}. Try again or check its service status."),
        });
    }
    response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE)
        .read_json::<Value>()
        .map_err(|_| "Provider returned invalid or oversized JSON.".into())
}

/// Read only known machine-readable error categories. Provider messages can echo secrets or prompt content.
fn rate_limit_message(error: &Value, retry_after: Option<u64>) -> String {
    let code = error
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = error
        .pointer("/error/type")
        .and_then(Value::as_str)
        .unwrap_or("");
    match code {
        "credit_balance_exhausted" => "HTTP 429: Your API credit balance is exhausted. Check the billing page for the organization/project that owns this key and add credits if needed. Retrying alone will not fix this.".into(),
        "project_spend_limit_exceeded" => "HTTP 429: This API project has reached its spending limit. Review the project's limit in your provider account; retrying alone will not fix this.".into(),
        "organization_spend_limit_exceeded" => "HTTP 429: Your API organization has reached its spending limit. Review its billing limits; retrying alone will not fix this.".into(),
        "organization_usage_limit_exceeded" => "HTTP 429: Your API organization has reached its approved usage limit. Check your provider account limits or contact its administrator/support.".into(),
        "rate_limit_exceeded" | "slow_down" => rate_wait(retry_after),
        _ if kind == "rate_limit_error" => rate_wait(retry_after),
        _ if kind == "insufficient_quota" || code == "insufficient_quota" =>
            "HTTP 429: API quota or billing access is unavailable. Check the credits and spending/usage limits for the organization/project that owns this key. Retrying alone will not fix this.".into(),
        _ => "Provider returned HTTP 429 without a recognized reason. Check API billing, quota and rate limits. Dewey cannot determine which limit was reached from this response.".into(),
    }
}
fn rate_wait(retry_after: Option<u64>) -> String {
    if let Some(seconds) = retry_after.filter(|s| *s > 0) {
        format!(
            "HTTP 429: The provider's temporary request/token rate limit was reached. Wait at least {seconds} seconds before sending again, or shorten the request."
        )
    } else {
        "HTTP 429: The provider's temporary request/token rate limit was reached. Wait briefly before sending again, or shorten the request. Check your account's model rate limits if it persists.".into()
    }
}

/// Discover rather than hard-code models. Follow Anthropic/Gemini pagination with a bounded page count.
pub fn models(profile: &Profile) -> Result<Vec<String>, String> {
    if profile.provider == Provider::ChatGpt {
        return super::plan::models(&profile.plan).map(|v| v.into_iter().map(|m| m.slug).collect());
    }
    models_with(profile, |url| request(profile, url, None))
}

fn models_with(
    profile: &Profile,
    mut fetch: impl FnMut(&str) -> Result<Value, String>,
) -> Result<Vec<String>, String> {
    if profile.provider == Provider::Azure {
        return Err("Azure uses your deployment name. Enter it manually; a public model catalog does not list your deployments.".into());
    }
    let mut found = Vec::new();
    let mut page = String::new();
    for _ in 0..20 {
        let mut url =
            url::Url::parse(&endpoint(profile, "models")?).map_err(|_| "Invalid endpoint.")?;
        match profile.provider {
            Provider::Gemini => {
                url.query_pairs_mut().append_pair("pageSize", "1000");
                if !page.is_empty() {
                    url.query_pairs_mut().append_pair("pageToken", &page);
                }
            }
            Provider::Anthropic => {
                url.query_pairs_mut().append_pair("limit", "1000");
                if !page.is_empty() {
                    url.query_pairs_mut().append_pair("after_id", &page);
                }
            }
            _ => {}
        }
        let value = fetch(url.as_str())?;
        let rows = if profile.provider == Provider::Gemini {
            value.get("models")
        } else {
            value.get("data")
        }
        .and_then(Value::as_array)
        .ok_or("The provider did not return a model catalog. Enter a model ID manually.")?;
        for row in rows {
            if profile.provider == Provider::Gemini
                && !row
                    .get("supportedGenerationMethods")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().any(|v| v == "generateContent"))
            {
                continue;
            }
            let id = if profile.provider == Provider::Gemini {
                row.get("name")
            } else {
                row.get("id")
            };
            if let Some(id) = id
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256)
            {
                found.push(id.strip_prefix("models/").unwrap_or(id).to_owned());
            }
        }
        let next = match profile.provider {
            Provider::Gemini => value.get("nextPageToken").and_then(Value::as_str),
            Provider::Anthropic if value.get("has_more") == Some(&Value::Bool(true)) => {
                value.get("last_id").and_then(Value::as_str)
            }
            _ => None,
        };
        match next {
            Some(next) if !next.is_empty() && next != page => page = next.into(),
            _ => break,
        }
    }
    found.sort();
    found.dedup();
    if found.is_empty() {
        return Err(
            "No chat models were returned. You can still enter a model ID manually.".into(),
        );
    }
    Ok(found)
}

pub(super) fn chat_body(
    profile: &Profile,
    messages: &[Message],
    context: &Value,
    audio: Option<&Audio>,
) -> Result<(String, Value), String> {
    if profile.provider == Provider::ChatGpt {
        return Err(
            "ChatGPT plan requests use OAuth and the Responses API, not an API key.".into(),
        );
    }
    if profile.model.trim().is_empty() {
        return Err("Choose or enter a model first.".into());
    }
    if audio.is_some() && (!profile.audio_input || !profile.provider.supports_audio()) {
        return Err(
            "Choose an audio-capable provider/model and enable audio input before attaching audio."
                .into(),
        );
    }
    let system = format!("{INSTRUCTIONS}\nCurrent song:\n{context}");
    let last = messages.len().saturating_sub(1);
    match profile.provider {
        Provider::Anthropic => Ok((
            endpoint(profile, "messages")?,
            json!({"model":profile.model.trim(),"max_tokens":8192,"system":system,
            "messages":messages.iter().map(|m|json!({"role":if m.assistant{"assistant"}else{"user"},"content":m.text})).collect::<Vec<_>>()}),
        )),
        Provider::Gemini => {
            let model = profile
                .model
                .trim()
                .strip_prefix("models/")
                .unwrap_or(profile.model.trim());
            if model.is_empty()
                || !model
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
            {
                return Err("Invalid Gemini model ID.".into());
            }
            let contents=messages.iter().enumerate().map(|(i,m)|{
                let mut parts=vec![json!({"text":m.text})];
                if i==last && let Some(a)=audio {parts.push(json!({"inlineData":{"mimeType":if a.format=="mp3"{"audio/mpeg"}else{"audio/wav"},"data":STANDARD.encode(&a.bytes)}}));}
                json!({"role":if m.assistant{"model"}else{"user"},"parts":parts})
            }).collect::<Vec<_>>();
            Ok((
                endpoint(profile, &format!("models/{model}:generateContent"))?,
                json!({"systemInstruction":{"parts":[{"text":system}]},"contents":contents,"generationConfig":{"maxOutputTokens":16384}}),
            ))
        }
        _ => {
            let mut list = vec![json!({"role":"system","content":system})];
            for (i, m) in messages.iter().enumerate() {
                let content = if i == last
                    && let Some(a) = audio
                {
                    json!([{"type":"text","text":m.text},{"type":"input_audio","input_audio":{"data":STANDARD.encode(&a.bytes),"format":a.format}}])
                } else {
                    Value::String(m.text.clone())
                };
                list.push(
                    json!({"role":if m.assistant{"assistant"}else{"user"},"content":content}),
                );
            }
            let mut body = json!({"model":profile.model.trim(),"messages":list});
            let limit = if matches!(profile.provider, Provider::OpenAi | Provider::Azure) {
                "max_completion_tokens"
            } else {
                "max_tokens"
            };
            body[limit] = json!(8192);
            Ok((endpoint(profile, "chat/completions")?, body))
        }
    }
}

pub(super) fn chat_text(provider: Provider, value: &Value) -> Result<String, String> {
    let text=match provider {
        Provider::Anthropic=> {
            if value.get("stop_reason").and_then(Value::as_str)==Some("max_tokens"){return Err("Reply was truncated. Ask for a shorter pattern.".into());}
            value.get("content").and_then(Value::as_array).map(|a|a.iter().filter_map(|v|v.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
        }
        Provider::Gemini=>{
            let candidate=value.pointer("/candidates/0");
            if candidate.and_then(|c|c.get("finishReason")).and_then(Value::as_str).is_some_and(|s|s!="STOP"){return Err("Gemini could not finish this reply. Try a shorter or different request.".into());}
            candidate.and_then(|c|c.pointer("/content/parts")).and_then(Value::as_array).map(|a|a.iter().filter(|p|p.get("thought")!=Some(&Value::Bool(true))).filter_map(|v|v.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
        }
        _=>{
            if value.pointer("/choices/0/finish_reason").and_then(Value::as_str).is_some_and(|s|s!="stop"){return Err("Provider did not finish a text reply. Try a shorter request or another chat model.".into());}
            value.pointer("/choices/0/message/content").and_then(Value::as_str).map(str::to_owned)
        }
    }.filter(|s|!s.trim().is_empty()).ok_or("Provider returned no text. Choose a model that supports chat replies.")?;
    Ok(text)
}
pub fn chat(
    profile: &Profile,
    messages: &[Message],
    context: &Value,
    audio: Option<&Audio>,
) -> Result<Answer, String> {
    if profile.provider == Provider::ChatGpt {
        if audio.is_some() {
            return Err("ChatGPT plan sign-in currently supports text only. Choose an audio-capable API-key model to analyze a reference recording.".into());
        }
        return super::plan::chat(&profile.plan, &profile.model, messages, context);
    }
    let (url, body) = chat_body(profile, messages, context, audio)?;
    if body.to_string().len() > 18 * 1024 * 1024 {
        return Err(
            "Conversation is too large. Clear chat or use a smaller audio reference.".into(),
        );
    }
    Answer::parse(&chat_text(
        profile.provider,
        &request(profile, &url, Some(body))?,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(provider: Provider) -> Profile {
        let mut p = Profile::new(provider);
        p.model = "test-model".into();
        p.api_key = "test-secret".into();
        p
    }
    fn messages() -> Vec<Message> {
        vec![Message {
            assistant: false,
            text: "Make a beat".into(),
        }]
    }
    #[test]
    fn protocols_route_messages_and_authentication_without_leaking_keys_into_bodies() {
        for provider in Provider::ALL {
            let mut p = profile(provider);
            if provider == Provider::ChatGpt {
                assert!(chat_body(&p, &messages(), &Value::Null, None).is_err());
                continue;
            }
            if provider == Provider::Azure {
                p.base_url = "https://test.openai.azure.com/openai/v1".into();
            }
            let (url, body) = chat_body(&p, &messages(), &json!({"tempo_bpm":120}), None).unwrap();
            assert!(!body.to_string().contains("test-secret"));
            assert!(url.starts_with(p.base_url.trim_end_matches('/')));
            match provider {
                Provider::Anthropic => {
                    assert!(url.ends_with("/messages"));
                    assert_eq!(body["messages"][0]["role"], "user");
                    assert_eq!(headers(&p)[0].0, "x-api-key");
                }
                Provider::Gemini => {
                    assert!(url.ends_with("/models/test-model:generateContent"));
                    assert_eq!(body["contents"][0]["parts"][0]["text"], "Make a beat");
                    assert_eq!(headers(&p)[0].0, "x-goog-api-key");
                }
                _ => {
                    assert!(url.ends_with("/chat/completions"));
                    assert_eq!(body["messages"][1]["content"], "Make a beat");
                }
            }
        }
    }
    #[test]
    fn audio_requires_explicit_model_support_and_uses_each_wire_format() {
        let audio = Audio {
            name: "reference.mp3".into(),
            format: "mp3",
            bytes: vec![1, 2, 3],
        };
        for provider in Provider::ALL {
            let mut p = profile(provider);
            assert!(chat_body(&p, &messages(), &Value::Null, Some(&audio)).is_err());
            p.audio_input = true;
            let result = chat_body(&p, &messages(), &Value::Null, Some(&audio));
            assert_eq!(result.is_ok(), provider.supports_audio());
            if let Ok((_, body)) = result {
                if provider == Provider::Gemini {
                    assert_eq!(
                        body["contents"][0]["parts"][1]["inlineData"]["data"],
                        "AQID"
                    );
                } else {
                    assert_eq!(
                        body["messages"][1]["content"][1]["input_audio"]["data"],
                        "AQID"
                    );
                }
            }
        }
    }
    #[test]
    fn live_catalogs_follow_pagination_and_keep_new_unknown_ids() {
        for provider in [Provider::Anthropic, Provider::Gemini] {
            let mut calls = 0;
            let found=models_with(&profile(provider),|url|{
                calls+=1;
                if calls==1 {
                    Ok(if provider==Provider::Gemini {json!({"models":[{"name":"models/new-model-z","supportedGenerationMethods":["generateContent"]},{"name":"models/embed","supportedGenerationMethods":["embedContent"]}],"nextPageToken":"next /?"})}
                    else{json!({"data":[{"id":"new-model-z"}],"has_more":true,"last_id":"new-model-z"})})
                }else{
                    let parsed=url::Url::parse(url).unwrap();let pairs=parsed.query_pairs().collect::<std::collections::HashMap<_,_>>();
                    if provider==Provider::Gemini{assert_eq!(pairs["pageToken"],"next /?");}else{assert_eq!(pairs["after_id"],"new-model-z");}
                    Ok(if provider==Provider::Gemini{json!({"models":[{"name":"models/new-model-a","supportedGenerationMethods":["generateContent"]}]})}else{json!({"data":[{"id":"new-model-a"}],"has_more":false})})
                }
            }).unwrap();
            assert_eq!(calls, 2);
            assert_eq!(found, vec!["new-model-a", "new-model-z"]);
        }
    }
    #[test]
    fn standard_catalog_is_sorted_deduplicated_and_failure_does_not_invent_models() {
        let found = models_with(&profile(Provider::Compatible), |_| {
            Ok(json!({"data":[{"id":"future-z"},{"id":"future-a"},{"id":"future-z"}]}))
        })
        .unwrap();
        assert_eq!(found, vec!["future-a", "future-z"]);
        assert!(models_with(&profile(Provider::OpenAi), |_| Ok(json!({"data":[]}))).is_err());
        assert!(
            models_with(&profile(Provider::Azure), |_| panic!(
                "Azure must use deployment name"
            ))
            .is_err()
        );
    }
    #[test]
    fn quota_and_temporary_rate_limits_have_distinct_actionable_errors() {
        let quota = rate_limit_message(
            &json!({"error":{"code":"insufficient_quota","message":"secret-api-key"}}),
            Some(10),
        );
        assert!(quota.contains("billing"));
        assert!(quota.contains("Retrying alone"));
        assert!(!quota.contains("secret-api-key"));
        let credit = rate_limit_message(
            &json!({"error":{"code":"credit_balance_exhausted","type":"insufficient_quota"}}),
            None,
        );
        assert!(credit.contains("credit balance"));
        let project = rate_limit_message(
            &json!({"error":{"code":"project_spend_limit_exceeded","type":"insufficient_quota"}}),
            None,
        );
        assert!(project.contains("project"));
        assert!(project.contains("spending limit"));
        let rate = rate_limit_message(&json!({"error":{"code":"rate_limit_exceeded"}}), Some(37));
        assert!(rate.contains("37 seconds"));
        assert!(!rate.contains("billing"));
        let unknown = rate_limit_message(&json!({"error":{"message":"private prompt text"}}), None);
        assert!(unknown.contains("cannot determine"));
        assert!(!unknown.contains("private prompt text"));
    }

    #[test]
    fn provider_replies_and_truncation_are_handled() {
        assert_eq!(
            chat_text(
                Provider::OpenAi,
                &json!({"choices":[{"message":{"content":"hello"},"finish_reason":"stop"}]})
            )
            .unwrap(),
            "hello"
        );
        assert_eq!(
            chat_text(
                Provider::Anthropic,
                &json!({"content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn"})
            )
            .unwrap(),
            "hello"
        );
        assert_eq!(chat_text(Provider::Gemini,&json!({"candidates":[{"content":{"parts":[{"thought":true,"text":"private reasoning"},{"text":"hello"}]},"finishReason":"STOP"}]})).unwrap(),"hello");
        assert!(
            chat_text(
                Provider::OpenAi,
                &json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]})
            )
            .is_err()
        );
        assert!(
            chat_text(
                Provider::Anthropic,
                &json!({"content":[{"text":"partial"}],"stop_reason":"max_tokens"})
            )
            .is_err()
        );
        assert!(chat_text(Provider::Gemini,&json!({"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]})).is_err());
    }
    #[test]
    fn an_endpoint_edit_cannot_send_a_previously_bound_key() {
        let mut p = profile(Provider::OpenAi);
        p.key_state.endpoint = base(&p).unwrap().into();
        p.base_url = "https://other.invalid/v1".into();
        let error = request(&p, "https://other.invalid/v1/models", None).unwrap_err();
        assert!(error.contains("URL changed"));
    }

    #[test]
    fn unsafe_or_ambiguous_urls_are_rejected_before_authentication() {
        let mut p = profile(Provider::Compatible);
        for url in [
            "http://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/v1#fragment",
            "file:///tmp/key",
        ] {
            p.base_url = url.into();
            assert!(base(&p).is_err(), "{url}");
        }
    }
}
