//! Ed25519 webhook signature verification.
//!
//! Discord signs every interaction webhook delivery with the
//! application's private key. The host application is expected to
//! verify each request before responding. The signature material is the
//! concatenation `timestamp_bytes || body`.
//!
//! Reference: <https://discord.com/developers/docs/interactions/receiving-and-responding#security-and-authorization>

use atomr_agents_channel_core::{ChannelError, Result};
use ed25519_dalek::{Signature, VerifyingKey};

pub(crate) const SIGNATURE_HEADER: &str = "x-signature-ed25519";
pub(crate) const TIMESTAMP_HEADER: &str = "x-signature-timestamp";

/// Verify an interactions webhook request body against the configured
/// public key.
pub(crate) fn verify(
    headers: &http::HeaderMap,
    body: &[u8],
    public_key_hex: &str,
) -> Result<()> {
    let sig_hex = headers
        .get(SIGNATURE_HEADER)
        .ok_or_else(|| ChannelError::webhook_verify("missing X-Signature-Ed25519"))?
        .to_str()
        .map_err(|_| ChannelError::webhook_verify("X-Signature-Ed25519 not ASCII"))?;
    let ts = headers
        .get(TIMESTAMP_HEADER)
        .ok_or_else(|| ChannelError::webhook_verify("missing X-Signature-Timestamp"))?
        .to_str()
        .map_err(|_| ChannelError::webhook_verify("X-Signature-Timestamp not ASCII"))?;

    let sig_bytes = hex::decode(sig_hex)
        .map_err(|_| ChannelError::webhook_verify("signature is not hex"))?;
    if sig_bytes.len() != 64 {
        return Err(ChannelError::webhook_verify("signature length must be 64"));
    }
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .expect("length checked above");
    let signature = Signature::from_bytes(&sig_arr);

    let pk_bytes = hex::decode(public_key_hex)
        .map_err(|_| ChannelError::webhook_verify("public key is not hex"))?;
    if pk_bytes.len() != 32 {
        return Err(ChannelError::webhook_verify(
            "public key length must be 32",
        ));
    }
    let pk_arr: [u8; 32] = pk_bytes
        .as_slice()
        .try_into()
        .expect("length checked above");
    let vk = VerifyingKey::from_bytes(&pk_arr)
        .map_err(|_| ChannelError::webhook_verify("invalid public key"))?;

    let mut msg = Vec::with_capacity(ts.len() + body.len());
    msg.extend_from_slice(ts.as_bytes());
    msg.extend_from_slice(body);

    vk.verify_strict(&msg, &signature)
        .map_err(|_| ChannelError::webhook_verify("signature mismatch"))
}
