# atomr-agents-coding-cli-harness-web

Axum + embedded SPA companion for the coding-cli harness.

## Run it

```bash
cargo run -p atomr-agents-coding-cli-harness-web
# default bind: 127.0.0.1:7300
```

Open `http://127.0.0.1:7300/` to see the SPA. Browse vendors, kick off
headless runs (live SSE event log), or open an interactive xterm.js
session against a tmux-wrapped CLI process.

## Routes

| Method | Path                                | Description                            |
| ------ | ----------------------------------- | -------------------------------------- |
| GET    | `/healthz`                          | Liveness probe                         |
| GET    | `/api/cli/vendors`                  | Vendors wired into the harness         |
| POST   | `/api/cli/runs`                     | Start a headless run, returns `run_id` |
| GET    | `/api/cli/runs`                     | Recent runs                            |
| GET    | `/api/cli/runs/:id`                 | One run                                |
| GET    | `/api/cli/runs/events`              | SSE of normalized events               |
| POST   | `/api/cli/sessions`                 | Start an interactive session           |
| GET    | `/api/cli/sessions`                 | Active interactive sessions            |
| DELETE | `/api/cli/sessions/:id`             | Stop an interactive session            |
| GET    | `/api/cli/sessions/:id/io` *(WS)*   | Terminal byte bridge                   |

## WebSocket protocol

* **Server → client**: binary frames carry raw PTY bytes (UTF-8 ANSI).
* **Client → server**:
  * Binary frame = stdin bytes (typed keys, paste).
  * Text frame `{"kind":"resize","cols":120,"rows":32}` resizes the PTY.

Run on port 7300 (next slot after deep-research's 7200).
