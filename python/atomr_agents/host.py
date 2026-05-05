"""Host-mode entry points.

Stub: full ``Harness.load`` / ``await harness.run(...)`` implementation
lands once the native module exposes harness construction. For now we
re-export the registry which is sufficient for testing artifact
publication / lookup from Python.
"""

from . import Registry

__all__ = ["Registry"]
