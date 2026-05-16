"""Tests for the YAML-frontmatter Markdown parser."""

from __future__ import annotations

from pathlib import Path

import pytest

from atomr_agents.agent_host.errors import MarkdownParseError
from atomr_agents.agent_host.markdown import parse_markdown, read_markdown

# All tests in this module require PyYAML for frontmatter parsing.
pytest.importorskip("yaml")


def test_empty_input_returns_empty_doc() -> None:
    doc = parse_markdown("")
    assert doc.is_empty()
    assert doc.frontmatter == {}
    assert doc.body == ""


def test_body_only_no_frontmatter() -> None:
    doc = parse_markdown("# Title\n\nbody text\n")
    assert doc.frontmatter == {}
    assert "Title" in doc.body
    assert "body text" in doc.body


def test_frontmatter_parsed_to_mapping() -> None:
    text = """---
identity: Alpha
style:
  tone: dry
  verbosity: 2
---

# Body

text
"""
    doc = parse_markdown(text)
    assert doc.frontmatter["identity"] == "Alpha"
    assert doc.frontmatter["style"]["tone"] == "dry"
    assert doc.frontmatter["style"]["verbosity"] == 2
    assert doc.body.startswith("# Body")


def test_unterminated_frontmatter_raises() -> None:
    text = "---\nidentity: oops\n# no closing fence\n"
    with pytest.raises(MarkdownParseError) as exc:
        parse_markdown(text, source_path=Path("/tmp/bad.md"))
    assert "no closing" in str(exc.value)


def test_non_mapping_frontmatter_raises() -> None:
    text = "---\n- just\n- a\n- list\n---\nbody\n"
    with pytest.raises(MarkdownParseError):
        parse_markdown(text)


def test_invalid_yaml_in_frontmatter_raises() -> None:
    # Indentation that yaml.safe_load rejects.
    text = "---\nkey: value\n  bad: indent\n---\nbody\n"
    with pytest.raises(MarkdownParseError):
        parse_markdown(text)


def test_read_markdown_missing_file_returns_empty(tmp_path: Path) -> None:
    doc = read_markdown(tmp_path / "no_such_file.md")
    assert doc.is_empty()
    assert doc.source_path == tmp_path / "no_such_file.md"


def test_leading_blank_lines_before_frontmatter() -> None:
    text = "\n\n---\nidentity: ok\n---\nbody\n"
    doc = parse_markdown(text)
    assert doc.frontmatter == {"identity": "ok"}
    assert doc.body == "body"


def test_only_frontmatter_no_body() -> None:
    text = "---\nidentity: foo\n---\n"
    doc = parse_markdown(text)
    assert doc.frontmatter == {"identity": "foo"}
    assert doc.body == ""
