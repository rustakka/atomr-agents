//! HMAC-SHA256 webhook signature verification.
//!
//! Meta's webhook delivery for WhatsApp signs each request body with
//! the app secret and ships the result in the `X-Hub-Signature-256`
//! header, formatted as `sha256=<lowercase hex>`. We recompute the
//! HMAC over the *raw* body bytes (no transformation, no JSON
//! reformatting) and compare in constant time.

use atomr_agents_channel_core::{ChannelError, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub(crate) const SIGNATURE_HEADER: &str = "x-hub-signature-256";
pub(crate) const SIGNATURE_PREFIX: &str = "sha256=";

type HmacSha256 = Hmac<Sha256>;

/// Compute the expected signature for `body` under `secret`, lowercase
/// hex (without the `sha256=` prefix). Exposed for tests.
pub(crate) fn compute_signature(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify the `X-Hub-Signature-256` header against `body`.
pub(crate) fn verify(
    headers: &http::HeaderMap,
    body: &[u8],
    app_secret: &[u8],
) -> Result<()> {
    let raw = headers
        .get(SIGNATURE_HEADER)
        .ok_or_else(|| ChannelError::webhook_verify("missing X-Hub-Signature-256"))?;
    let raw = raw
        .to_str()
        .map_err(|_| ChannelError::webhook_verify("X-Hub-Signature-256 not ASCII"))?;
    let sig_hex = raw
        .strip_prefix(SIGNATURE_PREFIX)
        .ok_or_else(|| ChannelError::webhook_verify("signature missing sha256= prefix"))?;
    let provided = hex::decode(sig_hex)
        .map_err(|_| ChannelError::webhook_verify("signature is not hex"))?;
    let expected_hex = compute_signature(app_secret, body);
    let expected =
        hex::decode(&expected_hex).expect("compute_signature returns valid hex");
    if provided.len() != expected.len() {
        return Err(ChannelError::webhook_verify("signature length mismatch"));
    }
    if provided.ct_eq(&expected).into() {
        Ok(())
    } else {
        Err(ChannelError::webhook_verify("signature mismatch"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue};

    fn header_map(sig: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(SIGNATURE_HEADER, HeaderValue::from_str(sig).unwrap());
        h
    }

    #[test]
    fn verify_happy_path() {
        let secret = b"shhh-app-secret";
        let body = br#"{"entry":[]}"#;
        let sig = format!("sha256={}", compute_signature(secret, body));
        verify(&header_map(&sig), body, secret).expect("signature must verify");
    }

    #[test]
    fn verify_rejects_bad_signature() {
        let secret = b"shhh-app-secret";
        let body = br#"{"entry":[]}"#;
        // Flip a byte in the computed signature.
        let good = compute_signature(secret, body);
        let mut bytes = good.into_bytes();
        bytes[0] = if bytes[0] == b'0' { b'1' } else { b'0' };
        let bad = String::from_utf8(bytes).unwrap();
        let sig = format!("sha256={bad}");
        let err = verify(&header_map(&sig), body, secret).expect_err("must reject");
        match err {
            ChannelError::WebhookVerify(_) => {}
            other => panic!("expected WebhookVerify, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_missing_prefix() {
        let secret = b"x";
        let body = b"{}";
        let sig = compute_signature(secret, body);
        let err = verify(&header_map(&sig), body, secret).expect_err("must reject");
        assert!(matches!(err, ChannelError::WebhookVerify(_)));
    }

    #[test]
    fn verify_rejects_missing_header() {
        let err =
            verify(&HeaderMap::new(), b"{}", b"x").expect_err("must reject missing header");
        assert!(matches!(err, ChannelError::WebhookVerify(_)));
    }
}
