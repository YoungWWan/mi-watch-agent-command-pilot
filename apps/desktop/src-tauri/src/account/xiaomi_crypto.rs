// Adapted from AstralSightStudios/AstroBox-Public src-tauri/src/auth.rs.
// https://github.com/AstralSightStudios/AstroBox-Public (AGPL-3.0).
use anyhow::Result;
use base64::{engine::general_purpose, Engine};
use rand::{rngs::OsRng, Rng};
use rc4::{cipher::StreamCipher, consts::U32, KeyInit, Rc4};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Url,
};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct MiAccountToken {
    pub ssecurity: String,
    pub service_token: String,
    pub c_user_id: String,
}

/// Generate Mi nonce: 8 random bytes + 4-byte minutes since epoch (big-endian).
fn generate_nonce(millis: u64) -> String {
    let mut rand_part = [0u8; 8];
    OsRng.fill(&mut rand_part);
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&rand_part);
    buf.extend_from_slice(&((millis / 60_000) as u32).to_be_bytes());
    general_purpose::STANDARD.encode(buf)
}

/// SHA-256( ssecurity_b64_dec + nonce_b64_dec ) → Base64.
fn calc_signed_nonce(ssecurity: &str, nonce: &str) -> Result<String> {
    let mut hasher = Sha256::default();
    hasher.update(&general_purpose::STANDARD.decode(ssecurity)?);
    hasher.update(&general_purpose::STANDARD.decode(nonce)?);
    Ok(general_purpose::STANDARD.encode(hasher.finalize()))
}

/// Deterministic Xiaomi signature builder (ASCII sort on keys).
fn generate_enc_signature(
    url_path: &str,
    method: &str,
    signed_nonce: &str,
    params: &HashMap<String, String>,
) -> String {
    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort(); // ASCII order
    let mut pieces = Vec::with_capacity(2 + keys.len() + 1);
    pieces.push(method.to_uppercase());
    pieces.push(url_path.to_owned());
    for k in keys {
        pieces.push(format!("{k}={}", params.get(k).unwrap()));
    }
    pieces.push(signed_nonce.to_owned());

    let raw = pieces.join("&");
    let mut sha1 = Sha1::default();
    sha1.update(raw.as_bytes());
    general_purpose::STANDARD.encode(sha1.finalize())
}

/// Encrypt payload parameters with RC4, respecting deterministic key order.
fn rc4_encrypt_params(
    signed_nonce: &str,
    params_plain: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    // Build a cipher, drop first 1024 bytes.
    let key_bytes = general_purpose::STANDARD.decode(signed_nonce)?;
    let key = rc4::Key::<U32>::from_slice(&key_bytes);
    let mut cipher = Rc4::<U32>::new(key);
    let mut drop_buf = [0u8; 1024];
    cipher.apply_keystream(&mut drop_buf);

    // Encrypt in deterministic order.
    let mut keys: Vec<&String> = params_plain.keys().collect();
    keys.sort(); // ASCII order

    let mut encrypted = HashMap::new();
    for k in keys {
        let mut data = params_plain.get(k).unwrap().as_bytes().to_vec();
        cipher.apply_keystream(&mut data);
        encrypted.insert(k.to_string(), general_purpose::STANDARD.encode(data));
    }
    Ok(encrypted)
}

/// Main encrypted API call.
/// `prefix` – path prefix that should be trimmed before signing (usually "")
pub async fn mi_service_call_encrypted(
    token: MiAccountToken,
    prefix: String,
    url: String,
    mut params_plain: HashMap<String, String>,
    ua: String,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let nonce = generate_nonce(millis);
    let signed_nonce = calc_signed_nonce(&token.ssecurity, &nonce)?;

    // Path for signature
    let url_parsed = Url::parse(&url)?;
    let url_path = url_parsed.path().trim_start_matches(&prefix).to_string();

    // 1. rc4_hash__ over *plain* params
    let rc4_hash = generate_enc_signature(&url_path, "POST", &signed_nonce, &params_plain);
    params_plain.insert("rc4_hash__".to_string(), rc4_hash);

    // 2. RC4-encrypt all fields
    let mut params_enc = rc4_encrypt_params(&signed_nonce, &params_plain)?;

    // 3. Final signature & nonce
    let sig = generate_enc_signature(&url_path, "POST", &signed_nonce, &params_enc);
    params_enc.insert("signature".into(), sig);
    params_enc.insert("_nonce".into(), nonce.clone());

    // 4. Headers & cookies
    let mut headers = HeaderMap::new();
    headers.insert("User-Agent", HeaderValue::from_str(&ua)?);
    headers.insert("region_tag", HeaderValue::from_static("cn"));
    headers.insert("HandleParams", HeaderValue::from_static("true"));

    let cookie_header = format!(
        "cUserId={}; serviceToken={}; locale=en_us;",
        token.c_user_id, token.service_token
    );

    // 5. Request
    let resp = client
        .post(url)
        .headers(headers)
        .header("Cookie", cookie_header)
        .form(&params_enc)
        .send()
        .await?
        .error_for_status()?;

    let body = resp.text().await?;

    // 6. Decrypt
    let key_bytes = general_purpose::STANDARD.decode(&signed_nonce)?;
    let key = rc4::Key::<U32>::from_slice(&key_bytes);
    let mut cipher = Rc4::<U32>::new(key);
    let mut drop_buf = [0u8; 1024];
    cipher.apply_keystream(&mut drop_buf);

    let mut data = general_purpose::STANDARD.decode(body.trim_matches('"'))?;
    cipher.apply_keystream(&mut data);
    Ok(String::from_utf8(data)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independently computed with Python hashlib and a reference RC4 implementation.
    #[test]
    fn health_protocol_known_vector() {
        let secret = general_purpose::STANDARD.encode((0u8..16).collect::<Vec<_>>());
        let nonce = general_purpose::STANDARD.encode((0u8..12).collect::<Vec<_>>());
        let signed = calc_signed_nonce(&secret, &nonce).unwrap();
        assert_eq!(signed, "Nz6YMO70n75ITJeXTWpiyXXLp6IBZ0yT0/hEJB0FDOg=");
        let path = "/app/v1/source/get_source_list";
        let mut params = HashMap::from([("data".into(), r#"{"page_size":50,"status":1}"#.into())]);
        let hash = generate_enc_signature(path, "POST", &signed, &params);
        assert_eq!(hash, "Xnwt/GAA+I0OhleGUDMSVXbYR+A=");
        params.insert("rc4_hash__".into(), hash);
        let enc = rc4_encrypt_params(&signed, &params).unwrap();
        assert_eq!(enc["data"], "Gjcp2s7UcCuQW/Gd6CvTStUrJeUjmCnJKNhK");
        assert_eq!(
            enc["rc4_hash__"],
            "JTr3ZUdVX62gxf3w3m8/ZuS/Pwb/iaa78XHCXw=="
        );
        assert_eq!(
            generate_enc_signature(path, "POST", &signed, &enc),
            "GXzAF9TA+nsvSWk712SUb4olFJ4="
        );
    }

    #[test]
    fn nonce_contains_random_bytes_and_big_endian_minutes() {
        let decoded = general_purpose::STANDARD
            .decode(generate_nonce(120_000))
            .unwrap();
        assert_eq!(decoded.len(), 12);
        assert_eq!(&decoded[8..], &2u32.to_be_bytes());
        assert!(calc_signed_nonce("invalid!", "invalid!").is_err());
    }
}
