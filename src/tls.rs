//! HTTPS natif : MCManager sert l'interface en TLS sans reverse proxy.
//!
//! Trois façons de l'activer (variables d'environnement, voir `docs/SERVER.md`) :
//!  - `MCMANAGER_TLS_CERT` + `MCMANAGER_TLS_KEY` : votre certificat (Let's Encrypt, par exemple) au format PEM ;
//!  - `MCMANAGER_TLS=self-signed` : un certificat auto-signé est créé (et renouvelé) dans `<données>/tls/` — le
//!    navigateur affichera un avertissement la première fois, mais la connexion est chiffrée ;
//!  - rien : HTTP simple (127.0.0.1 par défaut, sans exposition réseau).
//!
//! Option `MCMANAGER_HTTP_REDIRECT_PORT=80` : un petit serveur HTTP redirige vers HTTPS.
//! Quand TLS est actif, le cookie de session reçoit l'attribut `Secure`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use tokio_rustls::rustls::{Certificate, PrivateKey, ServerConfig};
use tokio_rustls::TlsAcceptor;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Le serveur est servi en HTTPS (utilisé pour marquer le cookie de session `Secure`).
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, PartialEq)]
pub enum TlsMode {
    Off,
    Files { cert: PathBuf, key: PathBuf },
    SelfSigned,
}

impl TlsMode {
    /// Mode demandé par l'environnement.
    pub fn from_env() -> TlsMode {
        Self::from_values(std::env::var("MCMANAGER_TLS_CERT").ok(), std::env::var("MCMANAGER_TLS_KEY").ok(), std::env::var("MCMANAGER_TLS").ok())
    }

    pub fn from_values(cert: Option<String>, key: Option<String>, mode: Option<String>) -> TlsMode {
        let (cert, key) = (cert.filter(|s| !s.trim().is_empty()), key.filter(|s| !s.trim().is_empty()));
        if let (Some(c), Some(k)) = (cert, key) {
            return TlsMode::Files { cert: PathBuf::from(c), key: PathBuf::from(k) };
        }
        match mode.as_deref().map(|m| m.trim().to_ascii_lowercase()).as_deref() {
            Some("self-signed") | Some("selfsigned") | Some("auto") | Some("1") | Some("true") | Some("on") => TlsMode::SelfSigned,
            _ => TlsMode::Off,
        }
    }
}

/// Noms (DNS et IP) à inscrire dans un certificat auto-signé : la machine, localhost, et `MCMANAGER_TLS_NAMES` (liste séparée par des virgules).
pub fn default_names(extra: Option<&str>, hostname: Option<String>) -> Vec<String> {
    let mut v = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
    if let Some(h) = hostname.filter(|h| !h.trim().is_empty()) {
        v.push(h);
    }
    for n in extra.unwrap_or("").split(',') {
        let n = n.trim();
        if !n.is_empty() && n.len() <= 253 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_' | '*')) {
            v.push(n.to_string());
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Certificat auto-signé (PEM certificat, PEM clé privée) valable 10 ans pour les noms donnés.
pub fn generate_self_signed(names: &[String]) -> Result<(String, String)> {
    use rcgen::{Certificate as RcCert, CertificateParams, DistinguishedName, DnType, SanType};
    let mut params = CertificateParams::default();
    params.subject_alt_names = names.iter().map(|n| match n.parse::<std::net::IpAddr>() { Ok(ip) => SanType::IpAddress(ip), Err(_) => SanType::DnsName(n.clone()) }).collect();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "MCManager");
    dn.push(DnType::OrganizationName, "MCManager (auto-signé)");
    params.distinguished_name = dn;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(3650);
    let cert = RcCert::from_params(params).context("génération du certificat")?;
    Ok((cert.serialize_pem().context("signature du certificat")?, cert.serialize_private_key_pem()))
}

fn read_certs(pem: &[u8]) -> Result<Vec<Certificate>> {
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(pem)).context("certificat illisible")?;
    if certs.is_empty() {
        anyhow::bail!("aucun certificat PEM trouvé");
    }
    Ok(certs.into_iter().map(Certificate).collect())
}

fn read_key(pem: &[u8]) -> Result<PrivateKey> {
    let mut rd = std::io::BufReader::new(pem);
    for item in rustls_pemfile::read_all(&mut rd).context("clé privée illisible")? {
        match item {
            rustls_pemfile::Item::PKCS8Key(k) | rustls_pemfile::Item::RSAKey(k) | rustls_pemfile::Item::ECKey(k) => return Ok(PrivateKey(k)),
            _ => {}
        }
    }
    anyhow::bail!("aucune clé privée PEM trouvée (PKCS#8, RSA ou EC)")
}

pub fn build_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<ServerConfig>> {
    let mut cfg = ServerConfig::builder().with_safe_defaults().with_no_client_auth().with_single_cert(read_certs(cert_pem)?, read_key(key_pem)?).context("le certificat et la clé ne correspondent pas")?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let cfg = Arc::new(cfg);
    // rustls 0.21 ne compare pas la clé au certificat à la construction : une poignée de main de test en mémoire échoue
    // ici, au démarrage, avec un message clair, plutôt qu'à chaque connexion d'un navigateur.
    handshake_selftest(cfg.clone()).context("le certificat et la clé ne correspondent pas")?;
    Ok(cfg)
}

/// Poignée de main TLS complète entre un client et un serveur en mémoire (le client accepte tout certificat).
fn handshake_selftest(cfg: Arc<ServerConfig>) -> Result<()> {
    use tokio_rustls::rustls::client::{ServerCertVerified, ServerCertVerifier};
    use tokio_rustls::rustls::{ClientConfig, ClientConnection, ServerConnection, ServerName};
    struct AcceptAll;
    impl ServerCertVerifier for AcceptAll {
        fn verify_server_cert(&self, _: &Certificate, _: &[Certificate], _: &ServerName, _: &mut dyn Iterator<Item = &[u8]>, _: &[u8], _: std::time::SystemTime) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
    }
    let client_cfg = ClientConfig::builder().with_safe_defaults().with_custom_certificate_verifier(Arc::new(AcceptAll)).with_no_client_auth();
    let mut client = ClientConnection::new(Arc::new(client_cfg), ServerName::try_from("localhost").map_err(|_| anyhow::anyhow!("nom"))?)?;
    let mut server = ServerConnection::new(cfg)?;
    for _ in 0..20 {
        let mut buf = Vec::new();
        client.write_tls(&mut buf)?;
        if !buf.is_empty() {
            server.read_tls(&mut buf.as_slice())?;
            server.process_new_packets()?;
        }
        let mut buf = Vec::new();
        server.write_tls(&mut buf)?;
        if !buf.is_empty() {
            client.read_tls(&mut buf.as_slice())?;
            client.process_new_packets()?;
        }
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok(());
        }
    }
    anyhow::bail!("poignée de main TLS incomplète")
}

/// Certificat auto-signé conservé dans `<données>/tls/` ; recréé s'il manque ou si la liste des noms a changé.
pub async fn load_or_create_self_signed(data_dir: &Path, names: &[String]) -> Result<(Vec<u8>, Vec<u8>)> {
    let dir = data_dir.join("tls");
    let (cp, kp, np) = (dir.join("cert.pem"), dir.join("key.pem"), dir.join("names.txt"));
    let wanted = names.join(",");
    if cp.is_file() && kp.is_file() && tokio::fs::read_to_string(&np).await.map(|n| n == wanted).unwrap_or(false) {
        return Ok((tokio::fs::read(&cp).await?, tokio::fs::read(&kp).await?));
    }
    let n = names.to_vec();
    let (cert, key) = tokio::task::spawn_blocking(move || generate_self_signed(&n)).await.map_err(|e| anyhow::anyhow!("erreur interne: {e}"))??;
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(&cp, &cert).await?;
    tokio::fs::write(&kp, &key).await?;
    tokio::fs::write(&np, &wanted).await?;
    crate::secrets::restrict_to_owner(&kp).await;
    Ok((cert.into_bytes(), key.into_bytes()))
}

/// Configuration TLS pour le mode demandé (`None` = HTTP simple).
pub async fn config_for(data_dir: &Path, mode: &TlsMode) -> Result<Option<Arc<ServerConfig>>> {
    match mode {
        TlsMode::Off => Ok(None),
        TlsMode::Files { cert, key } => {
            let c = tokio::fs::read(cert).await.with_context(|| format!("lecture de {}", cert.display()))?;
            let k = tokio::fs::read(key).await.with_context(|| format!("lecture de {}", key.display()))?;
            Ok(Some(build_config(&c, &k)?))
        }
        TlsMode::SelfSigned => {
            let names = default_names(std::env::var("MCMANAGER_TLS_NAMES").ok().as_deref(), sysinfo::System::host_name());
            let (c, k) = load_or_create_self_signed(data_dir, &names).await?;
            Ok(Some(build_config(&c, &k)?))
        }
    }
}

/// Sert `app` en HTTPS sur `listener` jusqu'à ce que `shutdown` se termine. WebSocket et SSE fonctionnent (mises à niveau HTTP/1.1).
pub async fn serve_tls<F>(app: Router, listener: tokio::net::TcpListener, tls: Arc<ServerConfig>, shutdown: F) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto::Builder;
    use hyper_util::service::TowerToHyperService;
    use tower::Service;

    ACTIVE.store(true, Ordering::SeqCst);
    let acceptor = TlsAcceptor::from(tls);
    let mut make = app.into_make_service_with_connect_info::<SocketAddr>();
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r { Ok(v) => v, Err(e) => { tracing::warn!("accept: {e}"); continue; } },
            _ = &mut shutdown => break,
        };
        let svc = match make.call(peer).await {
            Ok(s) => s,
            Err(never) => match never {},
        };
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            // Poignée de main TLS : un client en HTTP simple ou un certificat refusé échoue ici, sans bruit.
            let Ok(tls_stream) = tokio::time::timeout(std::time::Duration::from_secs(10), acceptor.accept(stream)).await.unwrap_or_else(|_| Err(std::io::Error::other("délai TLS"))) else { return };
            let io = TokioIo::new(tls_stream);
            let _ = Builder::new(TokioExecutor::new()).serve_connection_with_upgrades(io, TowerToHyperService::new(svc)).await;
        });
    }
    Ok(())
}

/// Petit serveur HTTP qui redirige tout vers `https://<hôte>:<port_https>`.
pub async fn serve_redirect(listener: tokio::net::TcpListener, https_port: u16) {
    use axum::extract::Request;
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    let app = Router::new().fallback(move |req: Request| async move {
        let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
        let host = host.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h).unwrap_or(host);
        let host: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':')).collect();
        let port = if https_port == 443 { String::new() } else { format!(":{https_port}") };
        let target = format!("https://{host}{port}{}", req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/"));
        (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, target)]).into_response()
    });
    let _ = axum::serve(listener, app).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;

    #[test]
    fn mode_selection_from_environment_values() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(TlsMode::from_values(None, None, None), TlsMode::Off);
        assert_eq!(TlsMode::from_values(s("c.pem"), s("k.pem"), None), TlsMode::Files { cert: "c.pem".into(), key: "k.pem".into() });
        assert_eq!(TlsMode::from_values(s("c.pem"), None, None), TlsMode::Off, "certificat sans clé : pas de TLS silencieux à moitié configuré");
        assert_eq!(TlsMode::from_values(None, None, s("self-signed")), TlsMode::SelfSigned);
        assert_eq!(TlsMode::from_values(None, None, s(" TRUE ")), TlsMode::SelfSigned);
        assert_eq!(TlsMode::from_values(None, None, s("0")), TlsMode::Off);
        assert_eq!(TlsMode::from_values(s("c"), s("k"), s("self-signed")), TlsMode::Files { cert: "c".into(), key: "k".into() }, "les fichiers l'emportent");
        assert_eq!(TlsMode::from_values(s(" "), s(" "), None), TlsMode::Off);
    }

    #[test]
    fn certificate_names() {
        let n = default_names(Some("mc.exemple.fr, 203.0.113.7 ,bad name,*.exemple.fr"), Some("monpc".into()));
        for want in ["localhost", "127.0.0.1", "::1", "monpc", "mc.exemple.fr", "203.0.113.7", "*.exemple.fr"] {
            assert!(n.contains(&want.to_string()), "{want} dans {n:?}");
        }
        assert!(!n.iter().any(|x| x.contains(' ')));
        assert_eq!(default_names(None, None).len(), 3);
    }

    #[tokio::test]
    async fn self_signed_certificate_is_created_reused_and_regenerated() {
        let d = std::env::temp_dir().join(format!("mcm-tls-{}", uuid::Uuid::new_v4()));
        let names = default_names(None, None);
        let (c1, k1) = load_or_create_self_signed(&d, &names).await.unwrap();
        assert!(String::from_utf8_lossy(&c1).contains("BEGIN CERTIFICATE") && String::from_utf8_lossy(&k1).contains("PRIVATE KEY"));
        build_config(&c1, &k1).expect("config TLS valide");
        let (c2, _) = load_or_create_self_signed(&d, &names).await.unwrap();
        assert_eq!(c1, c2, "même certificat tant que les noms ne changent pas");
        let (c3, _) = load_or_create_self_signed(&d, &default_names(Some("nouveau.exemple.fr"), None)).await.unwrap();
        assert_ne!(c1, c3, "nouveaux noms : nouveau certificat");
        // Une clé qui ne correspond pas au certificat est refusée avec un message clair.
        let (_, other_key) = generate_self_signed(&names).unwrap();
        assert!(build_config(&c1, other_key.as_bytes()).unwrap_err().to_string().contains("ne correspondent pas"));
        assert!(build_config(b"pas un pem", &k1).is_err() && build_config(&c1, b"pas un pem").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(d.join("tls/key.pem")).unwrap().permissions().mode() & 0o077, 0, "clé privée lisible par le propriétaire seulement");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn serves_https_and_refuses_plain_http() {
        let d = std::env::temp_dir().join(format!("mcm-tls2-{}", uuid::Uuid::new_v4()));
        let (c, k) = load_or_create_self_signed(&d, &default_names(None, None)).await.unwrap();
        let cfg = build_config(&c, &k).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let app = Router::new().route("/healthz", get(|| async { "ok" })).route("/ip", get(|axum::extract::ConnectInfo(a): axum::extract::ConnectInfo<SocketAddr>| async move { a.ip().to_string() }));
        let task = tokio::spawn(serve_tls(app, listener, cfg, async move { let _ = rx.await; }));

        // Certificat auto-signé : refusé par un client strict, accepté si on lui fait confiance (comme après l'avertissement du navigateur).
        let strict = reqwest::Client::new();
        assert!(strict.get(format!("https://127.0.0.1:{port}/healthz")).send().await.is_err(), "un client strict refuse l'auto-signé");
        let lax = reqwest::Client::builder().danger_accept_invalid_certs(true).build().unwrap();
        let r = lax.get(format!("https://127.0.0.1:{port}/healthz")).send().await.unwrap();
        assert_eq!((r.status().as_u16(), r.text().await.unwrap().as_str()), (200, "ok"));
        assert_eq!(lax.get(format!("https://127.0.0.1:{port}/ip")).send().await.unwrap().text().await.unwrap(), "127.0.0.1", "ConnectInfo (adresse du client) disponible derrière TLS");
        assert!(is_active());
        // HTTP simple sur le port TLS : pas de réponse exploitable.
        let plain = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build().unwrap();
        assert!(plain.get(format!("http://127.0.0.1:{port}/healthz")).send().await.map(|r| !r.status().is_success()).unwrap_or(true));
        let _ = tx.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), task).await.expect("arrêt propre");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn redirect_server_points_to_https() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(serve_redirect(l, 8443));
        let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let r = c.get(format!("http://127.0.0.1:{port}/api/servers?x=1")).header("Host", "mc.exemple.fr:80").send().await.unwrap();
        assert_eq!(r.status().as_u16(), 308);
        assert_eq!(r.headers()["location"], "https://mc.exemple.fr:8443/api/servers?x=1");
        let r = c.get(format!("http://127.0.0.1:{port}/")).header("Host", "evil.com/../x").send().await.unwrap();
        assert!(r.headers()["location"].to_str().unwrap().starts_with("https://evil.com..x:8443/") || r.headers()["location"].to_str().unwrap().starts_with("https://evil.com"));
    }
}
