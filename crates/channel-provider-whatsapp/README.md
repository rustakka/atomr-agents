# atomr-agents-channel-provider-whatsapp

WhatsApp Business Cloud API provider for the atomr-agents `channels` feature.

This crate implements [`ChannelProvider`](https://docs.rs/atomr-agents-channel-core)
against Meta's Cloud API. It is consumed by `atomr-agents-channel-harness`,
which mediates between agents (via `Callable`) and the messaging
transport.

## What it does

- **Outbound send.** `POST {api_base}/{phone_number_id}/messages` with a
  `messaging_product=whatsapp` JSON body. Text, image, audio, video, and
  document payloads are supported. `MessageContent::Mixed` is split
  across multiple sends: leading text is concatenated into a single text
  message, then each attachment is sent individually.
- **Media fetch.** Two-step: resolve the media id via `GET {api_base}/{media_id}`
  to obtain a short-lived download URL, then bearer-auth GET the URL for
  the bytes.
- **Webhook verification.** Validates the `X-Hub-Signature-256` header
  via constant-time HMAC-SHA256 against the configured app secret. The
  body is hashed verbatim — no normalisation.
- **Webhook parsing.** Walks `entry[].changes[].value.messages[]` and
  lifts each supported message into an `InboundMessage`. Unknown types
  (reactions, statuses, system events) are silently skipped.

WhatsApp is webhook-driven, so `start()` spawns a no-op task that
honours the cooperative `stop` flag — there is no gateway socket and
no long-poll loop.

## Configuration

The provider reads its config from `ChannelSpec::config` as a JSON
object:

| Field | Required | Notes |
|-------|----------|-------|
| `phone_number_id`     | yes | Numeric string assigned by Meta. Path component of the send URL. |
| `access_token`        | yes | Bearer token. Used for send + media. |
| `app_secret`          | yes | App secret. Used to verify webhook signatures. |
| `default_channel_id`  | yes | The channel id this provider is bound to. Inbound webhooks carry no channel context, so we stamp them with this. |
| `api_base`            | no  | Defaults to `https://graph.facebook.com/v18.0`. Useful for fakes. |

## Capability matrix

| Capability     | Supported |
|----------------|-----------|
| `text`         | yes       |
| `attachments`  | yes (image, audio, video, document) |
| `voice`        | no        |
| `reactions`    | no        |
| `typing`       | no        |
| `threads_native` | no      |

`Attachment::media_ref` is interpreted as an already-uploaded WhatsApp
media id (the value returned by Meta's `/media` upload endpoint).
Uploading new media is the caller's responsibility — this crate only
references existing ids on send.

## Webhook signature scheme

Meta signs each inbound webhook body with the configured app secret
and ships the result in `X-Hub-Signature-256` as `sha256=<lowercase hex>`.
`verify_webhook`:

1. Pulls the header (errors if missing).
2. Strips the `sha256=` prefix (errors if absent).
3. Hex-decodes the remainder.
4. Recomputes HMAC-SHA256(app_secret, raw_body).
5. Compares the two byte slices in constant time via `subtle::ConstantTimeEq`.

Hash the **raw bytes** of the request body — do not re-serialise the
JSON before verifying, or the signature will not match.

## Example

```rust
use atomr_agents_channel_provider_whatsapp::WhatsAppProvider;
use serde_json::json;

let provider = WhatsAppProvider::from_value(json!({
    "phone_number_id": "1234567890",
    "access_token": "EAAB...",
    "app_secret": "shhh",
    "default_channel_id": "channel-wa-prod",
})).expect("config parses");

assert_eq!(provider.kind().as_str(), "whatsapp");
```

`provider` is an `Arc<dyn ChannelProvider>` ready to register with the
channel harness.

## Tests

Unit tests cover HMAC verify (happy and tampered), webhook parse for
text + image + unknown types, config parsing, and clean shutdown of the
no-op `start` task. Send and media fetch are exercised by request-shape
unit tests; live HTTP is intentionally out of scope.
