# Default Docker image for the coding-cli harness's Antigravity CLI (agy) adapter.
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git curl ca-certificates tmux bash \
 && rm -rf /var/lib/apt/lists/*

# Installs the Go-based `agy` binary to ~/.local/bin (the legacy
# @google/gemini-cli is deprecated; sunset 2026-06-18).
RUN curl -fsSL https://antigravity.google/cli/install.sh | bash
ENV PATH="/root/.local/bin:${PATH}"

WORKDIR /workspace
ENTRYPOINT []
CMD ["agy", "--help"]
