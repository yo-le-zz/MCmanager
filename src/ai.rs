//! Mini AI assistant ("chatbox") that can suggest what to add, change, or
//! fix on a server, given a snapshot of its current state (loader, MC
//! version, installed addons, running status). Supports four providers -
//! Anthropic, OpenAI, Gemini, or a locally-running Ollama - the user
//! supplies their own API key (or, for Ollama, just a base URL, no key
//! needed) and MCManager talks to that provider directly; nothing is
//! proxied through any Anthropic-operated service.
//!
//! Security note (documented here rather than overclaimed to the user):
//! the API key is encrypted at rest with AES-256-GCM (`ai_config.json`
//! stores only ciphertext) using a key kept in a separate file
//! (`ai_key.bin`), both owner-only (`chmod 600` on Unix). This is a real
//! improvement over plain storage - a stray backup or sync of just the
//! config file, or a config pasted into a bug report, no longer leaks the
//! key - but it is *not* equivalent to an OS keychain/HSM: the decryption
//! key lives on the same disk, so anyone with read access to this
//! account's files (or root) can still decrypt it. That limitation is
//! inherent to a local desktop app with no external secret store, and is
//! surfaced to the user in the UI rather than hidden.
//!
//! Les outils (web, lecture et modification du serveur) et le streaming sont dans `ai_agent.rs`,
//! `ai_tools.rs`, `ai_stream.rs` et `ai_fetch.rs` : ce module garde la configuration chiffrée, la liste des modèles
//! et la recherche web.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::secrets::{self, EncryptedBlob};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    /// "anthropic" | "openai" | "gemini" | "ollama" | "omniroute"
    pub provider: String,
    /// Plaintext in memory only - see `StoredAiConfig` for the encrypted
    /// on-disk representation. Never (de)serialized directly to the config
    /// file; `load_config`/`save_config` handle the encrypt/decrypt step.
    #[serde(skip)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    /// Only used for provider == "ollama". Defaults to the standard local
    /// Ollama port if left empty.
    #[serde(default)]
    pub ollama_base_url: String,
    /// Only used for provider == "omniroute". OmniRoute (omniroute.online /
    /// github.com/diegosouzapw/OmniRoute) is a self-hosted, OpenAI-compatible
    /// AI gateway that fronts 300+ providers behind one endpoint - defaults
    /// to its standard local dashboard port, but can point at a remote
    /// instance the user runs elsewhere.
    #[serde(default)]
    pub omniroute_base_url: String,
    /// Mode des outils : `off` | `read` | `ask` (défaut, écritures confirmées) | `auto`. Vide = `ask`.
    #[serde(default)]
    pub tools_mode: String,
    /// Réservé aux tests : remplace l'adresse de l'API (Anthropic, OpenAI, Gemini) par un faux serveur local.
    #[serde(skip)]
    pub endpoint_override: String,
}

/// What actually lands in `ai_config.json`: everything in plain JSON except
/// the key, which is AES-256-GCM ciphertext (nonce + ciphertext, both
/// base64). Keeping this as a distinct type from `AiConfig` makes it
/// impossible to accidentally serialize a plaintext key to disk - there is
/// no `Serialize` path from `AiConfig.api_key` at all (see `#[serde(skip)]`
/// above), only the explicit encrypt step in `save_config`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredAiConfig {
    provider: String,
    model: String,
    ollama_base_url: String,
    #[serde(default)]
    omniroute_base_url: String,
    #[serde(default)]
    tools_mode: String,
    encrypted_key: Option<EncryptedBlob>,
}

impl AiConfig {
    pub(crate) fn ollama_url(&self) -> String {
        if self.ollama_base_url.trim().is_empty() {
            "http://127.0.0.1:11434".to_string()
        } else {
            self.ollama_base_url.trim_end_matches('/').to_string()
        }
    }

    /// OmniRoute's own docs use `http://localhost:20128/v1` as the default
    /// local base URL (its dashboard runs on a different port, 20129).
    pub(crate) fn omniroute_url(&self) -> String {
        if self.omniroute_base_url.trim().is_empty() {
            "http://127.0.0.1:20128/v1".to_string()
        } else {
            self.omniroute_base_url.trim_end_matches('/').to_string()
        }
    }

    /// Never send the real key back to the frontend once saved.
    pub fn masked_key(&self) -> String {
        if self.api_key.len() <= 8 {
            if self.api_key.is_empty() { String::new() } else { "•".repeat(self.api_key.len()) }
        } else {
            format!("{}…{}", &self.api_key[..4], &self.api_key[self.api_key.len() - 4..])
        }
    }
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ai_config.json")
}

pub async fn load_config(data_dir: &Path) -> AiConfig {
    let Ok(content) = tokio::fs::read_to_string(config_path(data_dir)).await else {
        return AiConfig::default();
    };
    let Ok(stored) = serde_json::from_str::<StoredAiConfig>(&content) else {
        return AiConfig::default();
    };
    let api_key = match &stored.encrypted_key {
        Some(blob) => match secrets::load_or_create_key(data_dir).await {
            Ok(key) => secrets::decrypt(&key, blob).unwrap_or_default(),
            Err(_) => String::new(),
        },
        None => String::new(),
    };
    AiConfig { provider: stored.provider, api_key, model: stored.model, ollama_base_url: stored.ollama_base_url, omniroute_base_url: stored.omniroute_base_url, tools_mode: stored.tools_mode, endpoint_override: String::new() }
}

pub async fn save_config(data_dir: &Path, cfg: &AiConfig) -> Result<()> {
    let encrypted_key = if cfg.api_key.is_empty() {
        None
    } else {
        let key = secrets::load_or_create_key(data_dir).await?;
        Some(secrets::encrypt(&key, &cfg.api_key)?)
    };
    let stored = StoredAiConfig {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        ollama_base_url: cfg.ollama_base_url.clone(),
        omniroute_base_url: cfg.omniroute_base_url.clone(),
        tools_mode: cfg.tools_mode.clone(),
        encrypted_key,
    };
    let path = config_path(data_dir);
    let json = serde_json::to_string_pretty(&stored)?;
    tokio::fs::write(&path, json).await?;
    secrets::restrict_to_owner(&path).await;
    Ok(())
}

/// Guesses the provider from the shape of a pasted API key, so the UI can
/// auto-select the right tab instead of making the user pick it manually
/// (per the "detection auto en fonction de la cle" request). Falls back to
/// leaving the current selection untouched when the shape isn't recognized
/// (e.g. Ollama needs no key at all).
pub fn detect_provider(api_key: &str) -> Option<&'static str> {
    let k = api_key.trim();
    if k.starts_with("sk-ant-") {
        Some("anthropic")
    } else if k.starts_with("sk-") {
        Some("openai")
    } else if k.starts_with("AIza") {
        Some("gemini")
    } else {
        None
    }
}

/// Lists models available for the given provider/key so the UI can offer a
/// dropdown instead of a free-text model name. Best-effort: providers that
/// don't expose (or reject) a list-models call fall back to a short, clearly
/// non-exhaustive default list rather than erroring out the whole page.
pub async fn list_models(http: &reqwest::Client, cfg: &AiConfig) -> Result<Vec<String>> {
    match cfg.provider.as_str() {
        "anthropic" => {
            let resp = http.get("https://api.anthropic.com/v1/models")
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01")
                .send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: Value = r.json().await.unwrap_or_default();
                    let ids: Vec<String> = body["data"].as_array().map(|a| {
                        a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()
                    }).unwrap_or_default();
                    if ids.is_empty() { Ok(fallback_models("anthropic")) } else { Ok(ids) }
                }
                _ => Ok(fallback_models("anthropic")),
            }
        }
        "openai" => {
            let resp = http.get("https://api.openai.com/v1/models")
                .bearer_auth(&cfg.api_key)
                .send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: Value = r.json().await.unwrap_or_default();
                    let mut ids: Vec<String> = body["data"].as_array().map(|a| {
                        a.iter().filter_map(|m| m["id"].as_str().map(String::from))
                            .filter(|id| id.starts_with("gpt-") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4"))
                            .collect()
                    }).unwrap_or_default();
                    ids.sort();
                    if ids.is_empty() { Ok(fallback_models("openai")) } else { Ok(ids) }
                }
                _ => Ok(fallback_models("openai")),
            }
        }
        "gemini" => {
            // La clé voyage dans un en-tête : une URL se retrouve dans les journaux et les proxys.
            let resp = http.get("https://generativelanguage.googleapis.com/v1beta/models").header("x-goog-api-key", &cfg.api_key).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: Value = r.json().await.unwrap_or_default();
                    let ids: Vec<String> = body["models"].as_array().map(|a| {
                        a.iter().filter_map(|m| m["name"].as_str().map(|s| s.trim_start_matches("models/").to_string()))
                            .filter(|n| n.contains("gemini"))
                            .collect()
                    }).unwrap_or_default();
                    if ids.is_empty() { Ok(fallback_models("gemini")) } else { Ok(ids) }
                }
                _ => Ok(fallback_models("gemini")),
            }
        }
        "ollama" => {
            let url = format!("{}/api/tags", cfg.ollama_url());
            let resp = http.get(&url).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: Value = r.json().await.unwrap_or_default();
                    let ids: Vec<String> = body["models"].as_array().map(|a| {
                        a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()
                    }).unwrap_or_default();
                    Ok(ids)
                }
                _ => anyhow::bail!("impossible de contacter Ollama sur {} - est-il lance ? (`ollama serve`)", cfg.ollama_url()),
            }
        }
        "omniroute" => {
            // OpenAI-compatible: GET /v1/models with a Bearer token.
            let url = format!("{}/models", cfg.omniroute_url());
            let resp = http.get(&url).bearer_auth(&cfg.api_key).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: Value = r.json().await.unwrap_or_default();
                    let mut ids: Vec<String> = body["data"].as_array().map(|a| {
                        a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()
                    }).unwrap_or_default();
                    ids.sort();
                    if !ids.contains(&"auto".to_string()) {
                        ids.insert(0, "auto".to_string());
                    }
                    Ok(ids)
                }
                _ => anyhow::bail!(
                    "impossible de contacter OmniRoute sur {} - verifiez qu'il tourne et que la cle API (Dashboard -> Endpoints) est correcte",
                    cfg.omniroute_url()
                ),
            }
        }
        other => anyhow::bail!("provider inconnu : {other}"),
    }
}

fn fallback_models(provider: &str) -> Vec<String> {
    // Not guaranteed current - the API-based list above is always tried
    // first. These only cover a total failure to reach the provider so the
    // UI still has *something* selectable and the user can type a custom
    // model id if theirs isn't listed.
    match provider {
        "anthropic" => vec!["claude-opus-4-1".into(), "claude-sonnet-4-5".into(), "claude-haiku-4-5".into()],
        "openai" => vec!["gpt-4.1".into(), "gpt-4.1-mini".into(), "o4-mini".into()],
        "gemini" => vec!["gemini-2.5-pro".into(), "gemini-2.5-flash".into()],
        _ => vec![],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

/// Enveloppe non streamée : exécute l'agent et renvoie la réponse complète (voir `ai_agent::run_collect`).
pub async fn chat(http: &reqwest::Client, cfg: &AiConfig, state: &crate::state::AppState, server_id: Option<uuid::Uuid>, context: &str, history: &[ChatMessage], message: &str) -> Result<String> {
    let _ = http; // le client HTTP de l'état est utilisé par l'agent
    let params = crate::ai_agent::AgentParams::new(state.clone(), cfg.clone(), server_id, context.to_string(), "local".into(), String::new());
    Ok(crate::ai_agent::run_collect(params, history.to_vec(), message.to_string()).await?.reply)
}

/// DuckDuckGo's HTML endpoint needs no API key, unlike Bing/Google/Serper -
/// the trade-off (documented, not hidden) is that it's an unofficial
/// scrape of public markup rather than a stable API, so it can break if
/// DuckDuckGo changes their page structure. To reduce (not eliminate) that
/// fragility: results are extracted via a small attribute-aware scan
/// (title + snippet + URL, not just a title substring), and a request or
/// parse failure falls back to Wikipedia's OpenSearch API - a stable, key-
/// free JSON endpoint - rather than returning nothing. Wikipedia is a weak
/// substitute for "latest plugin version"-style questions, and the tool
/// result says so explicitly so the model (and the user reading the
/// console) knows the answer may be less current than a live web result.
pub(crate) async fn tool_web_search(http: &reqwest::Client, query: &str) -> String {
    match duckduckgo_search(http, query).await {
        Some(results) if !results.is_empty() => {
            results.into_iter().enumerate()
                .map(|(i, r)| format!("{}. {} — {}\n   {}", i + 1, r.title, r.url, r.snippet))
                .collect::<Vec<_>>().join("\n")
        }
        _ => match wikipedia_fallback_search(http, query).await {
            Some(results) if !results.is_empty() => {
                let formatted = results.into_iter().enumerate()
                    .map(|(i, r)| format!("{}. {} — {}", i + 1, r.0, r.1))
                    .collect::<Vec<_>>().join("\n");
                format!(
                    "(recherche web indisponible - repli sur Wikipedia, qui peut etre moins a jour pour des sujets comme les versions de plugins)\n{formatted}"
                )
            }
            _ => "recherche impossible : ni DuckDuckGo ni le repli Wikipedia n'ont repondu. Reessayez, ou demandez a l'utilisateur de verifier la connexion internet du serveur.".to_string(),
        },
    }
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

async fn duckduckgo_search(http: &reqwest::Client, query: &str) -> Option<Vec<SearchResult>> {
    let resp = http.post("https://html.duckduckgo.com/html/").form(&[("q", query)]).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let html = resp.text().await.ok()?;

    // Each result sits in a `<div class="result ...">` block containing a
    // `result__a` link (title + href) and a `result__snippet` span. Scanning
    // block-by-block (rather than one global split on "result__a") keeps the
    // title/snippet/url grouped correctly even if DuckDuckGo reorders
    // attributes or adds classes, as long as those three markers still
    // appear together per result - a looser, more resilient assumption than
    // matching exact markup.
    let mut results = Vec::new();
    for block in html.split("class=\"result ").skip(1).take(6) {
        let href = extract_attr(block, "result__a", "href");
        let title = extract_between(block, "result__a", "</a>").map(|s| strip_tags(&s).trim().to_string());
        let snippet = extract_between(block, "result__snippet", "</a>")
            .or_else(|| extract_between(block, "result__snippet", "</div>"))
            .map(|s| strip_tags(&s).trim().to_string())
            .unwrap_or_default();
        if let (Some(title), Some(href)) = (title, href) {
            if !title.is_empty() {
                results.push(SearchResult { title, url: href, snippet });
            }
        }
    }
    Some(results)
}

/// Finds `attr="value"` on the tag that has `marker` somewhere in its class
/// list, within the first ~2KB after `marker` (results are short HTML
/// snippets, no need to scan the whole block).
fn extract_attr(block: &str, marker: &str, attr: &str) -> Option<String> {
    let start = block.find(marker)?;
    let window = &block[start..(start + 2000).min(block.len())];
    let tag_end = window.find('>')?;
    let tag = &window[..tag_end];
    let needle = format!("{attr}=\"");
    let attr_start = tag.find(&needle)? + needle.len();
    let attr_end = tag[attr_start..].find('"')?;
    Some(html_unescape(&tag[attr_start..attr_start + attr_end]))
}

/// Grabs the text between the tag containing `marker` and the given closing
/// tag, stripped of any nested markup.
fn extract_between(block: &str, marker: &str, close_tag: &str) -> Option<String> {
    let start = block.find(marker)?;
    let window = &block[start..];
    let content_start = window.find('>')? + 1;
    let content_end = window[content_start..].find(close_tag)?;
    Some(window[content_start..content_start + content_end].to_string())
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

async fn wikipedia_fallback_search(http: &reqwest::Client, query: &str) -> Option<Vec<(String, String)>> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=opensearch&format=json&limit=5&search={}",
        urlencoding_light(query)
    );
    let resp = http.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    let titles = body.get(1)?.as_array()?;
    let descriptions = body.get(2).and_then(|v| v.as_array());
    let mut out = Vec::new();
    for (i, t) in titles.iter().enumerate() {
        if let Some(title) = t.as_str() {
            let desc = descriptions.and_then(|d| d.get(i)).and_then(|v| v.as_str()).unwrap_or("").to_string();
            out.push((title.to_string(), desc));
        }
    }
    Some(out)
}

fn urlencoding_light(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

pub(crate) fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Encryption round-trip itself is covered by secrets::tests::roundtrip
    // now that the AES-GCM logic lives there, shared with ntfy.rs.

    #[test]
    fn masked_key_shape() {
        let cfg = AiConfig { provider: "anthropic".into(), api_key: "sk-ant-1234567890abcd".into(), model: String::new(), ollama_base_url: String::new(), ..Default::default() };
        let masked = cfg.masked_key();
        assert!(masked.contains('…'));
        assert!(!masked.contains("1234567890"));
    }

    #[test]
    fn omniroute_url_defaults_to_local_dashboard_port() {
        let cfg = AiConfig { provider: "omniroute".into(), api_key: String::new(), model: String::new(), ollama_base_url: String::new(), ..Default::default() };
        assert_eq!(cfg.omniroute_url(), "http://127.0.0.1:20128/v1");
        let cfg2 = AiConfig { omniroute_base_url: "https://my-omniroute.example.com/v1/".into(), ..cfg };
        assert_eq!(cfg2.omniroute_url(), "https://my-omniroute.example.com/v1");
    }
}
