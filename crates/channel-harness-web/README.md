# atomr-agents-channel-harness-web

Optional web companion for [`atomr-agents-channel-harness`]. Default bind:
`127.0.0.1:7400` — chosen to compose alongside the other `*-harness-web`
servers (`stt:7000`, `meetings:7100`, `coding-cli:7200`, `deep-research:7300`).

## Endpoints

| Method | Path                                        | Purpose                                        |
| ------ | ------------------------------------------- | ---------------------------------------------- |
| GET    | `/healthz`                                  | Liveness probe                                 |
| GET    | `/api/channels`                             | List attached channels                         |
| GET    | `/api/channels/:id`                         | Inspect a channel spec                         |
| DELETE | `/api/channels/:id`                         | Detach a provider                              |
| GET    | `/api/channels/:id/threads`                 | List threads on a channel                      |
| GET    | `/api/threads/:id`                          | Thread info                                    |
| DELETE | `/api/threads/:id`                          | Close a thread                                 |
| GET    | `/api/threads/:id/messages?limit=N`         | List recent messages                           |
| POST   | `/api/threads/:id/messages`                 | Admin send (bypasses bound target)             |
| POST   | `/webhook/:provider/:channel_id`            | Provider webhook entry (verified by provider)  |
| GET    | `/ws`                                       | Live `ChannelEvent` stream                     |

## Limitations

Attaching a provider and opening a thread require concrete Rust types
(`ChannelProvider` impl, `ThreadTarget` carrying a `Callable`). The REST
surface deliberately rejects `POST /api/channels` and `POST /api/channels/:id/threads`
with `405` — those operations are driven by the embedding application, then
surface here automatically.

## Running

```bash
cargo run -p atomr-agents-channel-harness-web
# customise the port
PORT=7401 cargo run -p atomr-agents-channel-harness-web
```

[`atomr-agents-channel-harness`]: ../channel-harness
