//! Mots de passe à usage unique basés sur le temps (TOTP, RFC 6238) : 6 chiffres, pas de 30 s, HMAC-SHA1.
//! C'est le profil que lisent toutes les applications d'authentification (Google Authenticator, Aegis, 2FAS, Authy...).
//!
//! Module pur, sans E/S : la gestion des comptes est dans `auth.rs`.

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const STEP_SECS: u64 = 30;
pub const DIGITS: u32 = 6;
/// Tolérance d'horloge : on accepte le pas précédent et le suivant (±30 s).
pub const WINDOW: i64 = 1;
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Secret de 160 bits (la taille recommandée par la RFC 4226), encodé en base32.
pub fn generate_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 8).div_ceil(5));
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Décodage tolérant : espaces, tirets, minuscules et `=` de bourrage acceptés.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let (mut buf, mut bits) = (0u32, 0u32);
    let mut out = Vec::new();
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '=') {
        let v = B32.iter().position(|&x| x as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push(((buf >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Code à `DIGITS` chiffres pour le compteur donné (RFC 4226, troncature dynamique).
pub fn code_for_counter(secret: &[u8], counter: u64) -> u32 {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("HMAC accepte toute taille de clé");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = ((h[off] as u32 & 0x7f) << 24) | ((h[off + 1] as u32) << 16) | ((h[off + 2] as u32) << 8) | h[off + 3] as u32;
    bin % 10u32.pow(DIGITS)
}

pub fn counter_at(unix_secs: u64) -> u64 {
    unix_secs / STEP_SECS
}

/// Vérifie un code saisi. Renvoie le compteur correspondant (à mémoriser pour refuser un rejeu : un même code ne
/// doit servir qu'une fois) ou `None`. `last_used` : dernier compteur accepté pour ce compte.
pub fn verify(secret_b32: &str, input: &str, unix_secs: u64, last_used: u64) -> Option<u64> {
    let digits: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() != DIGITS as usize || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let want: u32 = digits.parse().ok()?;
    let secret = base32_decode(secret_b32)?;
    let now = counter_at(unix_secs) as i64;
    let mut found = None;
    // Toutes les valeurs de la fenêtre sont calculées et comparées (pas de sortie anticipée sur le premier succès).
    for delta in -WINDOW..=WINDOW {
        let c = now + delta;
        if c < 0 {
            continue;
        }
        if code_for_counter(&secret, c as u64) == want && c as u64 > last_used {
            found = Some(c as u64);
        }
    }
    found
}

/// Lien `otpauth://` à ouvrir dans l'application (ou à transformer en QR code).
pub fn otpauth_uri(account: &str, issuer: &str, secret_b32: &str) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    format!("otpauth://totp/{}:{}?secret={secret_b32}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}", enc(issuer), enc(account), enc(issuer))
}

/// Codes de secours (usage unique) : 10 caractères hexadécimaux, présentés `xxxxx-xxxxx`.
pub fn generate_recovery_codes(n: usize) -> Vec<String> {
    use rand::RngCore;
    (0..n)
        .map(|_| {
            let mut b = [0u8; 5];
            rand::rngs::OsRng.fill_bytes(&mut b);
            let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
            format!("{}-{}", &h[..5], &h[5..])
        })
        .collect()
}

/// Forme canonique d'un code de secours saisi (minuscules, sans espaces ni tiret).
pub fn normalize_recovery(input: &str) -> String {
    input.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vecteurs de test de la RFC 6238 (annexe B), secret ASCII « 12345678901234567890 », tronqués à 6 chiffres.
    #[test]
    fn rfc6238_test_vectors() {
        let secret = b"12345678901234567890";
        for (t, expected) in [(59u64, 287082u32), (1111111109, 81804), (1111111111, 50471), (1234567890, 5924), (2000000000, 279037), (20000000000, 353130)] {
            assert_eq!(code_for_counter(secret, counter_at(t)), expected, "T={t}");
        }
    }

    #[test]
    fn base32_round_trip_and_tolerant_decoding() {
        assert_eq!(base32_encode(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(base32_decode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap(), b"12345678901234567890");
        assert_eq!(base32_decode("gezd gnbv-gy3t qojq gezd gnbv gy3t qojq==").unwrap(), b"12345678901234567890");
        assert!(base32_decode("1NVALID!").is_none() && base32_decode("").is_none());
        for len in 1..24 {
            let data: Vec<u8> = (0..len as u8).map(|x| x.wrapping_mul(37).wrapping_add(11)).collect();
            assert_eq!(base32_decode(&base32_encode(&data)).unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn generated_secrets_are_160_bits_and_unique() {
        let (a, b) = (generate_secret(), generate_secret());
        assert_eq!(a.len(), 32);
        assert_eq!(base32_decode(&a).unwrap().len(), 20);
        assert_ne!(a, b);
    }

    #[test]
    fn verification_window_replay_and_input_hygiene() {
        let secret = base32_encode(b"12345678901234567890");
        let t = 1_111_111_109u64; // compteur 37037036, code 081804
        assert_eq!(verify(&secret, "081804", t, 0), Some(counter_at(t)));
        assert_eq!(verify(&secret, "081 804", t, 0), Some(counter_at(t)), "espaces ignorés");
        assert_eq!(verify(&secret, "081804", t + 30, 0), Some(counter_at(t)), "pas précédent accepté");
        assert_eq!(verify(&secret, "081804", t - 30, 0), Some(counter_at(t)), "pas suivant accepté");
        assert_eq!(verify(&secret, "081804", t + 90, 0), None, "trop ancien");
        assert_eq!(verify(&secret, "081804", t, counter_at(t)), None, "rejeu du même code refusé");
        assert_eq!(verify(&secret, "081804", t, counter_at(t) - 1), Some(counter_at(t)));
        for bad in ["", "12345", "1234567", "abcdef", "08180a", "081804 ", "٠٨١٨٠٤"] {
            let r = verify(&secret, bad, t, 0);
            assert!(r.is_none() || bad == "081804 ", "{bad:?}");
        }
        assert_eq!(verify("pas du base32 !", "081804", t, 0), None);
    }

    #[test]
    fn otpauth_uri_is_escaped() {
        let u = otpauth_uri("Jean Dupont@exemple", "MC Manager", "ABC234");
        assert_eq!(u, "otpauth://totp/MC%20Manager:Jean%20Dupont%40exemple?secret=ABC234&issuer=MC%20Manager&algorithm=SHA1&digits=6&period=30");
    }

    #[test]
    fn recovery_codes_are_distinct_and_normalizable() {
        let codes = generate_recovery_codes(8);
        assert_eq!(codes.len(), 8);
        let set: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(set.len(), 8);
        for c in &codes {
            assert_eq!(c.len(), 11);
            assert_eq!(normalize_recovery(c).len(), 10);
            assert_eq!(normalize_recovery(&c.to_uppercase()), normalize_recovery(c));
        }
    }
}
