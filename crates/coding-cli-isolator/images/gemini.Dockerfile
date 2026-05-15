# Default Docker image for the coding-cli harness's Gemini CLI adapter.
FROM node:20-bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git curl ca-certificates tmux \
 && rm -rf /var/lib/apt/lists/*

RUN npm install -g @google/gemini-cli

WORKDIR /workspace
ENTRYPOINT []
CMD ["gemini", "--help"]
