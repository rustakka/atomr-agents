"""Facade over :mod:`atomr_agents._native.ingest`.

Re-exports the loader / splitter / KV cache / pipeline surface from
``atomr-agents-ingest``:

- ``Document`` value type
- ``Loader`` handle + factories (``text_loader``, ``markdown_loader``,
  ``csv_loader``, ``json_loader``, ``loader_from_factory``)
- ``Splitter`` handle + factories (``recursive_character_splitter``,
  ``token_splitter``, ``markdown_header_splitter``, ``code_splitter``,
  ``semantic_splitter``, ``splitter_from_factory``)
- ``CodeLang`` enum
- ``KvCache`` handle + ``in_memory_kv_cache``, ``kv_cache_from_factory``
- ``cached_embedder`` (wraps an Embedder + KvCache)
- ``IngestPipeline`` builder + ``ingest`` free function
"""

from ._native import ingest as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
