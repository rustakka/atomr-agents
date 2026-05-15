# Default Docker image for the coding-cli harness's Claude Code adapter.
# Provides the `claude` binary, `tmux`, and a workspace bind-mount point.
FROM node:20-bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git curl ca-certificates tmux \
 && rm -rf /var/lib/apt/lists/*

RUN npm install -g @anthropic-ai/claude-code

WORKDIR /workspace
ENTRYPOINT []
CMD ["claude", "--help"]
