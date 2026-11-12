//! `web_fetch` de l'assistant, durci contre le SSRF.
//!
//! Le modèle (ou une page qu'il a lue) peut demander n'importe quelle URL : sans garde-fous, l'outil
//! donnerait accès à `127.0.0.1`, aux métadonnées cloud (`169.254.169.254`) ou au réseau local de
//! l'utilisateur. Règles appliquées :
//!  - http/https uniquement, sans identifiants dans l'URL, ports 80/443/8080/8443 seulement ;
//!  - noms `localhost`, `*.local`, `*.internal`, `*.lan` et noms sans point refusés ;
//!  - le DNS est résolu **ici**, toute adresse non publique fait refuser la requête, et l'adresse
//!    vérifiée est **épinglée** (`ClientBuilder::resolve`) pour empêcher un changement de réponse DNS
//!    entre la vérification et la connexion ;
//!  - les redirections sont suivies à la main (4 au plus) et chaque cible est revérifiée ;
//!  - corps limité à 512 Ko, types texte / json / xml uniquement.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use futures_util::StreamExt;
use url::{Host, Url};

pub const MAX_BODY: usize = 512 * 1024;
pub const MAX_REDIRECTS: usize = 4;
const ALLOWED_PORTS: [u16; 4] = [80, 443, 8080, 8443];
const BLOCKED_SUFFIXES: [&str; 5] = [".local", ".internal", ".lan", ".localhost", ".home.arpa"];

#[cfg(test)]
thread_local! {
    /// Réservé aux tests : autorise un faux serveur sur 127.0.0.1 (jamais compilé en release).
    pub static ALLOW_LOCAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn allow_local() -> bool {
    #[cfg(test)]
    {
        ALLOW_LOCAL.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        false
    }
}

/// Adresse joignable depuis Internet. S'appuie sur `network::is_public_ip` et ajoute ce qu'elle ne couvre pas
/// (multicast, réservé, benchmarking, IPv4 « mappée » dans une IPv6, NAT64).
pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => is_public_v4(v),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public_v4(&v4);
            }
            let s = v.segments();
            // 64:ff9b::/96 (NAT64) embarque une IPv4 : on la contrôle.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                let o = v.octets();
                return is_public_v4(&Ipv4Addr::new(o[12], o[13], o[14], o[15]));
            }
            crate::network::is_public_ip(ip) && !v.is_multicast() && !is_documentation_v6(v)
        }
    }
}

fn is_public_v4(v: &Ipv4Addr) -> bool {
    let o = v.octets();
    crate::network::is_public_ip(&IpAddr::V4(*v))
        && !v.is_multicast()
        && o[0] < 240
        && !(o[0] == 192 && o[1] == 0 && o[2] == 0)           // 192.0.0.0/24
        && !(o[0] == 198 && (o[1] & 0xFE) == 18)              // 198.18.0.0/15 (benchmarking)
}

fn is_documentation_v6(v: &Ipv6Addr) -> bool {
    let s = v.segments();
    s[0] == 0x2001 && s[1] == 0x0db8
}

/// Valide une URL avant toute connexion réseau (pur, sans DNS).
pub fn validate_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "URL invalide".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("seules les URL http et https sont autorisées".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("les identifiants dans l'URL sont refusés".into());
    }
    let host = url.host().ok_or("URL sans hôte")?;
    let local_ok = allow_local();
    if !local_ok {
        let port = url.port_or_known_default().unwrap_or(0);
        if !ALLOWED_PORTS.contains(&port) {
            return Err(format!("port {port} refusé (autorisés : 80, 443, 8080, 8443)"));
        }
    }
    match host {
        Host::Ipv4(ip) => {
            if !local_ok && !is_public(&IpAddr::V4(ip)) {
                return Err("adresse non publique refusée".into());
            }
        }
        Host::Ipv6(ip) => {
            if !local_ok && !is_public(&IpAddr::V6(ip)) {
                return Err("adresse non publique refusée".into());
            }
        }
        Host::Domain(name) => {
            let n = name.trim_end_matches('.').to_ascii_lowercase();
            if !local_ok && (n == "localhost" || !n.contains('.') || BLOCKED_SUFFIXES.iter().any(|s| n.ends_with(s))) {
                return Err("nom d'hôte local refusé".into());
            }
        }
    }
    Ok(url)
}

/// Client pour un saut : redirections désactivées, DNS résolu et épinglé sur une adresse publique.
async fn pinned_client(url: &Url) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent("mcmanager-assistant/1.1 (+https://github.com/yo-le-zz/MCmanager)");
    if let Some(Host::Domain(name)) = url.host() {
        let port = url.port_or_known_default().unwrap_or(443);
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name, port)).await.map_err(|e| format!("résolution DNS impossible : {e}"))?.collect();
        if addrs.is_empty() {
            return Err("nom d'hôte introuvable".into());
        }
        // Une seule adresse non publique suffit à refuser : un attaquant peut mélanger les réponses.
        if !allow_local() && addrs.iter().any(|a| !is_public(&a.ip())) {
            return Err("le nom d'hôte pointe vers une adresse non publique : refusé".into());
        }
        b = b.resolve(name, addrs[0]);
    }
    b.build().map_err(|e| e.to_string())
}

fn is_text_type(ct: &str) -> bool {
    let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    ct.starts_with("text/") || ct == "application/json" || ct == "application/xml" || ct == "application/xhtml+xml" || ct.ends_with("+json") || ct.ends_with("+xml")
}

/// Récupère une page et la renvoie en texte brut (balises retirées), sans la tronquer à la taille d'affichage.
pub async fn fetch(raw_url: &str) -> Result<String, String> {
    let mut url = validate_url(raw_url)?;
    for _ in 0..=MAX_REDIRECTS {
        let client = pinned_client(&url).await?;
        let resp = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "text/html,text/plain,application/json,application/xml;q=0.9")
            .send()
            .await
            .map_err(|e| format!("requête impossible : {}", e.without_url()))?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or("redirection sans destination")?;
            let next = url.join(loc).map_err(|_| "redirection invalide".to_string())?;
            url = validate_url(next.as_str())?; // chaque saut est revérifié
            continue;
        }
        if !status.is_success() {
            return Err(format!("la page a répondu avec le statut {status}"));
        }
        let ct = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if !is_text_type(&ct) {
            return Err(format!("type de contenu non pris en charge : {}", if ct.is_empty() { "inconnu" } else { &ct }));
        }
        let mut body: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("lecture interrompue : {}", e.without_url()))?;
            let room = MAX_BODY - body.len();
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if body.len() >= MAX_BODY {
                break;
            }
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        let plain = if ct.to_ascii_lowercase().contains("html") { html_to_text(&text) } else { text };
        return Ok(format!("{url}\n{plain}"));
    }
    Err("trop de redirections".into())
}

/// Retire `<script>` et `<style>` (contenu compris), puis les balises restantes.
pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript"] {
        s = drop_blocks(&s, tag);
    }
    crate::ai::strip_tags(&s)
}

fn drop_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(start) = lower[pos..].find(&open).map(|i| i + pos) {
        out.push_str(&html[pos..start]);
        match lower[start..].find(&close) {
            Some(end) => pos = start + end + close.len(),
            None => return out, // bloc jamais refermé : on coupe tout le reste
        }
    }
    out.push_str(&html[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn rejected(u: &str) -> bool {
        validate_url(u).is_err()
    }

    #[test]
    fn refuses_non_http_schemes_and_credentials() {
        for u in ["file:///etc/passwd", "ftp://example.com/x", "gopher://example.com", "javascript:alert(1)", "data:text/html,hi", "pas une url", "https://user:pw@example.com/", "https://user@example.com/"] {
            assert!(rejected(u), "{u}");
        }
    }

    #[test]
    fn refuses_loopback_private_and_metadata_addresses() {
        for u in [
            "http://127.0.0.1/", "http://127.0.0.1:8080/", "http://[::1]/", "http://0.0.0.0/", "http://10.0.0.5/", "http://192.168.1.1/", "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data/", "http://100.64.0.1/", "http://[fe80::1]/", "http://[fd00::1]/", "http://[::ffff:127.0.0.1]/", "http://[::ffff:10.0.0.1]/",
            "http://2130706433/", "http://0x7f000001/", "http://127.1/", "http://224.0.0.1/", "http://198.18.0.1/", "http://[64:ff9b::7f00:1]/", "http://[ff02::1]/",
        ] {
            assert!(rejected(u), "{u}");
        }
    }

    #[test]
    fn refuses_local_names_and_odd_ports() {
        for u in ["http://localhost/", "http://LOCALHOST./", "http://app.localhost/", "http://nas.local/", "http://db.internal/", "http://box.lan/", "http://routeur/", "https://example.com:22/", "https://example.com:6379/", "http://example.com:25565/"] {
            assert!(rejected(u), "{u}");
        }
    }

    #[test]
    fn accepts_normal_public_urls() {
        for u in ["https://modrinth.com/mod/lithium", "http://example.com/", "https://example.com:8443/x?y=1", "https://93.184.216.34/", "https://[2606:4700:4700::1111]/"] {
            assert!(validate_url(u).is_ok(), "{u}");
        }
    }

    #[test]
    fn strips_scripts_and_styles() {
        let t = html_to_text("<html><head><style>a{color:red}</style><script>evil()</script></head><body><p>Bonjour <b>monde</b></p><SCRIPT>x</SCRIPT></body></html>");
        assert_eq!(t, "Bonjour monde");
        assert_eq!(html_to_text("avant<script>jamais refermé"), "avant");
    }

    /// Mini serveur HTTP : renvoie `responses` dans l'ordre, une par connexion.
    async fn serve(responses: Vec<String>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for r in responses {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(r.as_bytes()).await;
            }
        });
        port
    }
    fn http(status: &str, headers: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn fetches_text_and_limits_body_size() {
        ALLOW_LOCAL.with(|c| c.set(true));
        let big = "a".repeat(MAX_BODY + 5000);
        let port = serve(vec![http("200 OK", "Content-Type: text/plain\r\n", "bonjour"), http("200 OK", "Content-Type: text/plain\r\n", &big)]).await;
        let ok = fetch(&format!("http://127.0.0.1:{port}/")).await.unwrap();
        assert!(ok.ends_with("bonjour"));
        let port2 = port; // même écouteur : seconde réponse
        let cut = fetch(&format!("http://127.0.0.1:{port2}/")).await.unwrap();
        assert!(cut.len() <= MAX_BODY + 64, "corps non limité : {}", cut.len());
    }

    #[tokio::test]
    async fn refuses_binary_and_error_statuses() {
        ALLOW_LOCAL.with(|c| c.set(true));
        let port = serve(vec![http("200 OK", "Content-Type: application/zip\r\n", "PK"), http("404 Not Found", "Content-Type: text/plain\r\n", "x")]).await;
        assert!(fetch(&format!("http://127.0.0.1:{port}/")).await.unwrap_err().contains("type de contenu"));
        assert!(fetch(&format!("http://127.0.0.1:{port}/")).await.unwrap_err().contains("404"));
    }

    #[tokio::test]
    async fn redirects_are_followed_and_rechecked() {
        // Test réel : la cible de la redirection est une adresse privée, interdite même quand « local » est autorisé
        // pour le premier saut ? Non : on teste avec allow_local désactivé après le premier saut via une boucle.
        ALLOW_LOCAL.with(|c| c.set(true));
        let port = serve(vec![http("302 Found", "Location: /final\r\n", ""), http("200 OK", "Content-Type: text/html\r\n", "<p>fin</p>")]).await;
        let t = fetch(&format!("http://127.0.0.1:{port}/start")).await.unwrap();
        assert!(t.ends_with("fin") && t.contains("/final"));
        // Boucle infinie de redirections : plafonnée.
        let loop_resp: Vec<String> = (0..=MAX_REDIRECTS + 1).map(|_| http("302 Found", "Location: /again\r\n", "")).collect();
        let p2 = serve(loop_resp).await;
        assert!(fetch(&format!("http://127.0.0.1:{p2}/")).await.unwrap_err().contains("trop de redirections"));
    }

    #[tokio::test]
    async fn redirect_to_private_target_is_refused() {
        // Sans l'autorisation de test, la cible privée est refusée par validate_url (chaque saut est contrôlé).
        ALLOW_LOCAL.with(|c| c.set(false));
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
        let base = Url::parse("https://example.com/a").unwrap();
        let next = base.join("http://127.0.0.1:80/admin").unwrap();
        assert!(validate_url(next.as_str()).is_err());
        let rel = base.join("//10.1.2.3/x").unwrap();
        assert!(validate_url(rel.as_str()).is_err());
    }
}
