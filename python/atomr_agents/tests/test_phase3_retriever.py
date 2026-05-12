"""Smoke tests for Phase 3.1 retriever bindings."""

from __future__ import annotations

import pytest

from atomr_agents import retriever as ret


def test_document_round_trip():
    d = ret.Document(id="x", page_content="hello")
    assert d.id == "x"
    assert d.page_content == "hello"


def test_bm25_retriever_constructs():
    docs = [
        ret.Document(id="1", page_content="rust is a systems language"),
        ret.Document(id="2", page_content="python is dynamic"),
    ]
    r = ret.bm25_retriever(docs, top_k=2)
    assert r is not None


@pytest.mark.asyncio
async def test_bm25_retrieves_matching_doc():
    docs = [
        ret.Document(id="1", page_content="rust is a systems language"),
        ret.Document(id="2", page_content="python is dynamic"),
    ]
    r = ret.bm25_retriever(docs, top_k=1)
    out = await r.retrieve("rust")
    assert len(out) >= 1
