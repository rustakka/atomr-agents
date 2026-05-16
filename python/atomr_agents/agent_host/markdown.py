"""Markdown+YAML-frontmatter parser — thin facade over ``_native.host.MarkdownDoc``."""

from __future__ import annotations

import dataclasses
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from atomr_agents._native import host as _host

from .errors import MarkdownParseError

__all__ = ["MarkdownDoc", "parse_markdown", "read_markdown"]

FRONTMATTER_DELIM = "---"


@dataclass(frozen=True)
class MarkdownDoc:
    frontmatter: dict[str, Any] = field(default_factory=dict)
    body: str = ""
    source_path: Path | None = None

    def is_empty(self) -> bool:
        return not self.frontmatter and not self.body


def _strip_leading_blank_lines(text: str) -> str:
    stripped = text.lstrip("﻿")
    lines = stripped.splitlines(keepends=True)
    i = 0
    while i < len(lines) and lines[i].strip() == "":
        i += 1
    return "".join(lines[i:])


def _detect_unterminated_frontmatter(text: str) -> bool:
    lines = text.splitlines()
    if not lines or lines[0].strip() != FRONTMATTER_DELIM:
        return False
    for j in range(1, len(lines)):
        if lines[j].strip() == FRONTMATTER_DELIM:
            return False
    return True


def parse_markdown(text: str, *, source_path: Path | None = None) -> MarkdownDoc:
    if not text.strip():
        return MarkdownDoc(source_path=source_path)
    normalized = _strip_leading_blank_lines(text)
    if _detect_unterminated_frontmatter(normalized):
        raise MarkdownParseError(
            "frontmatter opened with '---' but no closing '---' was found",
            path=str(source_path) if source_path else None,
        )
    try:
        native = _host.MarkdownDoc.parse_str(normalized)
    except RuntimeError as exc:
        raise MarkdownParseError(str(exc), path=str(source_path) if source_path else None) from exc
    return MarkdownDoc(
        frontmatter=dict(native.frontmatter or {}),
        body=(native.body or "").strip(),
        source_path=source_path,
    )


def read_markdown(path: Path) -> MarkdownDoc:
    if not path.is_file():
        return MarkdownDoc(source_path=path)
    return parse_markdown(path.read_text(encoding="utf-8"), source_path=path)
