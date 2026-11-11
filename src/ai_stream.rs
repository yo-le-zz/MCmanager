//! Types communs de l'assistant et décodage des flux de réponse des fournisseurs.
//!
//! Chaque fournisseur parle son propre format de flux (SSE Anthropic, SSE OpenAI, SSE Gemini, NDJSON Ollama),
//! mais la boucle de l'agent (`ai_agent.rs`) ne voit qu'une suite de [`Event`] et un [`Turn`] final.
//! Les décodeurs sont volontairement purs (une ligne en entrée, des événements en sortie) pour être testés
//! sans réseau, avec des lignes coupées n'importe où.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

/// Événements envoyés au navigateur (SSE sur `/api/ai/chat/stream`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Delta { text: String },
    ToolStart { id: String, name: String, label_fr: String, label_en: String, args: Value, needs_approval: bool },
    ToolEnd { id: String, name: String, ok: bool, denied: bool, summary: String },
    Error { message: String },
    Done,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// Historique neutre, converti au format de chaque fournisseur à chaque requête.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    User(String),
    Assistant { text: String, calls: Vec<ToolCall> },
    Tool { call_id: String, name: String, content: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    pub text: String,
    pub calls: Vec<ToolCall>,
}

/// Description d'un outil offerte au modèle.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub properties: Value,
    pub required: &'static [&'static str],
}

impl ToolSpec {
    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": self.properties, "required": self.required })
    }
}

/// Erreur HTTP d'un fournisseur (permet de détecter « ce modèle ne gère pas les outils »).
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiError {}

pub fn error_message(body: &Value) -> String {
    body["error"]["message"].as_str().or_else(|| body["error"].as_str()).or_else(|| body["message"].as_str()).unwrap_or("erreur inconnue").to_string()
}

// ───────────────────────── lecture du flux ─────────────────────────

const MAX_LINE: usize = 4 * 1024 * 1024;

/// Lit une réponse HTTP en flux et appelle `on_line` pour chaque ligne complète. Les morceaux peuvent être coupés
/// n'importe où (même au milieu d'un caractère UTF-8) : on ne découpe que sur `\n`, en octets.
/// Échoue si le fournisseur reste muet plus de `idle`, ou si le navigateur s'est déconnecté.
pub async fn read_lines<F>(resp: reqwest::Response, idle: Duration, tx: &UnboundedSender<Event>, mut on_line: F) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let next = tokio::time::timeout(idle, stream.next()).await.map_err(|_| anyhow!("le fournisseur n'a rien envoyé depuis {} s : flux abandonné", idle.as_secs().max(1)))?;
        let Some(chunk) = next else { break };
        buf.extend_from_slice(&chunk.context("flux interrompu")?);
        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            on_line(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']))?;
        }
        if buf.len() > MAX_LINE {
            bail!("ligne de flux trop longue");
        }
        if tx.is_closed() {
            bail!("navigateur déconnecté");
        }
    }
    if !buf.is_empty() {
        on_line(String::from_utf8_lossy(&buf).trim())?;
    }
    Ok(())
}

// ───────────────────────── accumulateur ─────────────────────────

#[derive(Debug, Default, Clone)]
struct Partial {
    id: String,
    name: String,
    args_buf: String,
    args: Option<Value>,
}

/// Réassemble texte et appels d'outils à partir des fragments d'un flux ; le texte part au navigateur au fil de l'eau.
pub struct Acc<'a> {
    tx: &'a UnboundedSender<Event>,
    text: String,
    calls: Vec<Partial>,
    slots: HashMap<u64, usize>,
    pub done: bool,
}

impl<'a> Acc<'a> {
    pub fn new(tx: &'a UnboundedSender<Event>) -> Self {
        Acc { tx, text: String::new(), calls: Vec::new(), slots: HashMap::new(), done: false }
    }

    fn emit(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        let _ = self.tx.send(Event::Delta { text: text.to_string() });
    }

    /// Emplacement d'un appel d'outil désigné par son index dans le flux (créé au premier fragment).
    fn slot(&mut self, index: u64) -> &mut Partial {
        let next = self.calls.len();
        let i = *self.slots.entry(index).or_insert(next);
        if i == self.calls.len() {
            self.calls.push(Partial::default());
        }
        &mut self.calls[i]
    }

    pub fn finish(self) -> Turn {
        let calls = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, p)| !p.name.is_empty())
            .map(|(n, p)| {
                let args = p.args.unwrap_or_else(|| if p.args_buf.trim().is_empty() { json!({}) } else { serde_json::from_str(&p.args_buf).unwrap_or_else(|_| json!({})) });
                ToolCall { id: if p.id.is_empty() { format!("call_{n}") } else { p.id }, name: p.name, args }
            })
            .collect();
        Turn { text: self.text, calls }
    }
}

fn data_payload(line: &str) -> Option<&str> {
    let d = line.strip_prefix("data:")?;
    Some(d.strip_prefix(' ').unwrap_or(d))
}

fn parse_or_skip(payload: &str) -> Option<Value> {
    serde_json::from_str(payload).ok()
}

/// Anthropic : `content_block_start` / `content_block_delta` (`text_delta`, `input_json_delta`) / `message_stop`.
pub fn feed_anthropic(acc: &mut Acc, line: &str) -> Result<()> {
    let Some(payload) = data_payload(line) else { return Ok(()) };
    let Some(v) = parse_or_skip(payload) else { return Ok(()) };
    match v["type"].as_str().unwrap_or("") {
        "content_block_start" => {
            let b = &v["content_block"];
            if b["type"] == "tool_use" {
                let p = acc.slot(v["index"].as_u64().unwrap_or(0));
                p.id = b["id"].as_str().unwrap_or("").to_string();
                p.name = b["name"].as_str().unwrap_or("").to_string();
            }
        }
        "content_block_delta" => {
            let d = &v["delta"];
            match d["type"].as_str().unwrap_or("") {
                "text_delta" => acc.emit(d["text"].as_str().unwrap_or("")),
                "input_json_delta" => {
                    let frag = d["partial_json"].as_str().unwrap_or("");
                    acc.slot(v["index"].as_u64().unwrap_or(0)).args_buf.push_str(frag);
                }
                _ => {}
            }
        }
        "message_stop" => acc.done = true,
        "error" => bail!("{}", v["error"]["message"].as_str().unwrap_or("erreur du fournisseur")),
        _ => {}
    }
    Ok(())
}

/// OpenAI et OmniRoute : `choices[0].delta.content`, `delta.tool_calls[index]`, `[DONE]`.
pub fn feed_openai(acc: &mut Acc, line: &str) -> Result<()> {
    let Some(payload) = data_payload(line) else { return Ok(()) };
    if payload.trim() == "[DONE]" {
        acc.done = true;
        return Ok(());
    }
    let Some(v) = parse_or_skip(payload) else { return Ok(()) };
    if v.get("error").is_some() {
        bail!("{}", error_message(&v));
    }
    let delta = &v["choices"][0]["delta"];
    if let Some(t) = delta["content"].as_str() {
        acc.emit(t);
    }
    if let Some(calls) = delta["tool_calls"].as_array() {
        for c in calls {
            let p = acc.slot(c["index"].as_u64().unwrap_or(0));
            if let Some(id) = c["id"].as_str() {
                p.id = id.to_string();
            }
            if let Some(n) = c["function"]["name"].as_str() {
                p.name.push_str(n);
            }
            if let Some(a) = c["function"]["arguments"].as_str() {
                p.args_buf.push_str(a);
            }
        }
    }
    Ok(())
}

/// Gemini (`:streamGenerateContent?alt=sse`) : chaque événement est un fragment complet de candidat.
pub fn feed_gemini(acc: &mut Acc, line: &str) -> Result<()> {
    let Some(payload) = data_payload(line) else { return Ok(()) };
    let Some(v) = parse_or_skip(payload) else { return Ok(()) };
    if v.get("error").is_some() {
        bail!("{}", error_message(&v));
    }
    if let Some(parts) = v["candidates"][0]["content"]["parts"].as_array() {
        for part in parts {
            if part["thought"] == true {
                continue; // résumé de réflexion : pas pour l'utilisateur
            }
            if let Some(t) = part["text"].as_str() {
                acc.emit(t);
            }
            if let Some(name) = part["functionCall"]["name"].as_str() {
                let index = 10_000 + acc.calls.len() as u64; // un functionCall arrive toujours complet
                let p = acc.slot(index);
                p.name = name.to_string();
                p.args = Some(part["functionCall"]["args"].clone()).filter(|a| a.is_object()).or(Some(json!({})));
            }
        }
    }
    if v["candidates"][0]["finishReason"].is_string() {
        acc.done = true;
    }
    Ok(())
}

/// Ollama : un objet JSON par ligne (pas de préfixe `data:`).
pub fn feed_ollama(acc: &mut Acc, line: &str) -> Result<()> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }
    let Some(v) = parse_or_skip(line) else { return Ok(()) };
    if let Some(e) = v["error"].as_str() {
        bail!("{e}");
    }
    if let Some(t) = v["message"]["content"].as_str() {
        acc.emit(t);
    }
    if let Some(calls) = v["message"]["tool_calls"].as_array() {
        for c in calls {
            let index = 10_000 + acc.calls.len() as u64;
            let p = acc.slot(index);
            p.name = c["function"]["name"].as_str().unwrap_or("").to_string();
            let args = &c["function"]["arguments"];
            p.args = Some(match args {
                Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                Value::Object(_) => args.clone(),
                _ => json!({}),
            });
        }
    }
    if v["done"] == true {
        acc.done = true;
    }
    Ok(())
}

// ───────────────────────── messages par fournisseur ─────────────────────────

fn args_object(v: &Value) -> Value {
    if v.is_object() { v.clone() } else { json!({}) }
}

pub fn anthropic_messages(msgs: &[Msg]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in msgs {
        match m {
            Msg::User(t) => out.push(json!({ "role": "user", "content": t })),
            Msg::Assistant { text, calls } => {
                let mut blocks = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
                for c in calls {
                    blocks.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": args_object(&c.args) }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "(vide)" }));
                }
                out.push(json!({ "role": "assistant", "content": blocks }));
            }
            Msg::Tool { call_id, content, .. } => {
                let block = json!({ "type": "tool_result", "tool_use_id": call_id, "content": content });
                // Tous les résultats d'un même tour vont dans un seul message utilisateur.
                if let Some(last) = out.last_mut().filter(|l| l["role"] == "user" && l["content"].is_array()) {
                    last["content"].as_array_mut().expect("tableau").push(block);
                } else {
                    out.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }
    }
    out
}

pub fn anthropic_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema() })).collect())
}

/// Format « chat completions » OpenAI (aussi utilisé par OmniRoute).
pub fn openai_messages(system: &str, msgs: &[Msg]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": system })];
    for m in msgs {
        match m {
            Msg::User(t) => out.push(json!({ "role": "user", "content": t })),
            Msg::Assistant { text, calls } => {
                let mut v = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
                if !calls.is_empty() {
                    v["tool_calls"] = Value::Array(calls.iter().map(|c| json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": args_object(&c.args).to_string() } })).collect());
                }
                out.push(v);
            }
            Msg::Tool { call_id, content, .. } => out.push(json!({ "role": "tool", "tool_call_id": call_id, "content": content })),
        }
    }
    out
}

pub fn openai_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(tools.iter().map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema() } })).collect())
}

pub fn gemini_contents(msgs: &[Msg]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in msgs {
        match m {
            Msg::User(t) => out.push(json!({ "role": "user", "parts": [{ "text": t }] })),
            Msg::Assistant { text, calls } => {
                let mut parts = Vec::new();
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                for c in calls {
                    parts.push(json!({ "functionCall": { "name": c.name, "args": args_object(&c.args) } }));
                }
                if parts.is_empty() {
                    parts.push(json!({ "text": "(vide)" }));
                }
                out.push(json!({ "role": "model", "parts": parts }));
            }
            Msg::Tool { name, content, .. } => {
                let part = json!({ "functionResponse": { "name": name, "response": { "result": content } } });
                if let Some(last) = out.last_mut().filter(|l| l["role"] == "user" && l["parts"][0]["functionResponse"].is_object()) {
                    last["parts"].as_array_mut().expect("tableau").push(part);
                } else {
                    out.push(json!({ "role": "user", "parts": [part] }));
                }
            }
        }
    }
    out
}

pub fn gemini_tools(tools: &[ToolSpec]) -> Value {
    let decls: Vec<Value> = tools
        .iter()
        .map(|t| {
            let mut d = json!({ "name": t.name, "description": t.description });
            // Gemini refuse un schéma d'objet sans propriété : on omet alors `parameters`.
            if t.properties.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                d["parameters"] = t.schema();
            }
            d
        })
        .collect();
    json!([{ "function_declarations": decls }])
}

pub fn ollama_messages(system: &str, msgs: &[Msg]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": system })];
    for m in msgs {
        match m {
            Msg::User(t) => out.push(json!({ "role": "user", "content": t })),
            Msg::Assistant { text, calls } => {
                let mut v = json!({ "role": "assistant", "content": text });
                if !calls.is_empty() {
                    v["tool_calls"] = Value::Array(calls.iter().map(|c| json!({ "function": { "name": c.name, "arguments": args_object(&c.args) } })).collect());
                }
                out.push(v);
            }
            Msg::Tool { name, content, .. } => out.push(json!({ "role": "tool", "tool_name": name, "content": content })),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn texts(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> String {
        let mut s = String::new();
        while let Ok(e) = rx.try_recv() {
            if let Event::Delta { text } = e {
                s.push_str(&text);
            }
        }
        s
    }

    #[test]
    fn anthropic_stream_text_and_tool() {
        let (tx, mut rx) = unbounded_channel();
        let mut acc = Acc::new(&tx);
        for l in [
            "event: message_start", r#"data: {"type":"message_start","message":{}}"#, "",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Je "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"regarde."}}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"install_addon","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"project_"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"id\":\"lithium\"}"}}"#,
            r#"data: {"type":"message_stop"}"#,
        ] {
            feed_anthropic(&mut acc, l).unwrap();
        }
        assert!(acc.done);
        let turn = acc.finish();
        assert_eq!(texts(&mut rx), "Je regarde.");
        assert_eq!(turn.calls, vec![ToolCall { id: "toolu_1".into(), name: "install_addon".into(), args: json!({"project_id":"lithium"}) }]);
    }

    #[test]
    fn openai_stream_text_and_split_tool_arguments() {
        let (tx, mut rx) = unbounded_channel();
        let mut acc = Acc::new(&tx);
        for l in [
            r#"data: {"choices":[{"delta":{"role":"assistant","content":"Salut"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read_console_logs","arguments":""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"lines\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":20}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"list_backups","arguments":"{}"}}]}}]}"#,
            "data: [DONE]",
        ] {
            feed_openai(&mut acc, l).unwrap();
        }
        assert!(acc.done);
        let turn = acc.finish();
        assert_eq!(texts(&mut rx), "Salut");
        assert_eq!(turn.calls.len(), 2);
        assert_eq!(turn.calls[0].args, json!({"lines":20}));
        assert_eq!(turn.calls[1].name, "list_backups");
    }

    #[test]
    fn gemini_stream_text_and_function_call() {
        let (tx, mut rx) = unbounded_channel();
        let mut acc = Acc::new(&tx);
        feed_gemini(&mut acc, r#"data: {"candidates":[{"content":{"parts":[{"text":"Un instant "},{"text":"pensée","thought":true}]}}]}"#).unwrap();
        feed_gemini(&mut acc, r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"web_search","args":{"query":"lithium"}}}]},"finishReason":"STOP"}]}"#).unwrap();
        assert!(acc.done);
        let turn = acc.finish();
        assert_eq!(texts(&mut rx), "Un instant ");
        assert_eq!(turn.calls[0].name, "web_search");
        assert_eq!(turn.calls[0].args, json!({"query":"lithium"}));
        assert_eq!(turn.calls[0].id, "call_0");
    }

    #[test]
    fn ollama_ndjson_text_and_tool_call() {
        let (tx, mut rx) = unbounded_channel();
        let mut acc = Acc::new(&tx);
        for l in [
            r#"{"message":{"role":"assistant","content":"Ok "},"done":false}"#,
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_server_status","arguments":{}}}]},"done":false}"#,
            r#"{"message":{"role":"assistant","content":""},"done":true}"#,
        ] {
            feed_ollama(&mut acc, l).unwrap();
        }
        assert!(acc.done);
        let turn = acc.finish();
        assert_eq!(texts(&mut rx), "Ok ");
        assert_eq!(turn.calls[0].name, "get_server_status");
    }

    #[test]
    fn provider_errors_surface() {
        let (tx, _rx) = unbounded_channel();
        let mut acc = Acc::new(&tx);
        assert!(feed_anthropic(&mut acc, r#"data: {"type":"error","error":{"message":"surcharge"}}"#).unwrap_err().to_string().contains("surcharge"));
        assert!(feed_openai(&mut acc, r#"data: {"error":{"message":"quota"}}"#).unwrap_err().to_string().contains("quota"));
        assert!(feed_gemini(&mut acc, r#"data: {"error":{"message":"clé refusée"}}"#).is_err());
        assert!(feed_ollama(&mut acc, r#"{"error":"model not found"}"#).is_err());
        // Lignes sans intérêt ou invalides : ignorées sans erreur.
        feed_anthropic(&mut acc, ": ping").unwrap();
        feed_openai(&mut acc, "data: pas du json").unwrap();
        feed_ollama(&mut acc, "").unwrap();
    }

    fn sample_history() -> Vec<Msg> {
        vec![
            Msg::User("installe lithium".into()),
            Msg::Assistant { text: "ok".into(), calls: vec![ToolCall { id: "c1".into(), name: "install_addon".into(), args: json!({"project_id":"lithium"}) }, ToolCall { id: "c2".into(), name: "list_backups".into(), args: json!({}) }] },
            Msg::Tool { call_id: "c1".into(), name: "install_addon".into(), content: "fait".into() },
            Msg::Tool { call_id: "c2".into(), name: "list_backups".into(), content: "aucune".into() },
        ]
    }

    #[test]
    fn tool_results_round_trip_in_every_format() {
        let h = sample_history();
        let a = anthropic_messages(&h);
        assert_eq!(a.len(), 3);
        assert_eq!(a[1]["content"][1]["type"], "tool_use");
        assert_eq!(a[2]["content"].as_array().unwrap().len(), 2, "résultats fusionnés dans un seul message");
        assert_eq!(a[2]["content"][0]["tool_use_id"], "c1");

        let o = openai_messages("sys", &h);
        assert_eq!(o[0]["role"], "system");
        assert_eq!(o[2]["tool_calls"][0]["function"]["arguments"], "{\"project_id\":\"lithium\"}");
        assert_eq!(o[3]["tool_call_id"], "c1");

        let g = gemini_contents(&h);
        assert_eq!(g[1]["role"], "model");
        assert_eq!(g[1]["parts"][1]["functionCall"]["name"], "install_addon");
        assert_eq!(g[2]["parts"].as_array().unwrap().len(), 2);
        assert_eq!(g[2]["parts"][0]["functionResponse"]["name"], "install_addon");

        let l = ollama_messages("sys", &h);
        assert_eq!(l[2]["tool_calls"][0]["function"]["arguments"]["project_id"], "lithium");
        assert_eq!(l[3]["role"], "tool");
    }

    #[test]
    fn gemini_omits_empty_parameters() {
        let t = [ToolSpec { name: "a", description: "d", properties: json!({}), required: &[] }, ToolSpec { name: "b", description: "d", properties: json!({"q":{"type":"string"}}), required: &["q"] }];
        let v = gemini_tools(&t);
        assert!(v[0]["function_declarations"][0].get("parameters").is_none());
        assert!(v[0]["function_declarations"][1].get("parameters").is_some());
    }
}
