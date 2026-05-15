# Default Docker image for the coding-cli harness's Codex CLI adapter.
FROM rust:1.83-slim-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git curl ca-certificates tmux nodejs npm \
 && rm -rf /var/lib/apt/lists/*

# Install Codex CLI from npm (the open-source distribution).
RUN npm install -g @openai/codex

WORKDIR /workspace
ENTRYPOINT []
CMD ["codex", "--help"]
