"""Internal parse/embed execution service for Knoted.

The package implements the worker-facing protocol in
``api/intelligence.openapi.yaml``: an HTTP API (:mod:`knoted_intelligence.app`),
a durable SQLite ledger (:mod:`knoted_intelligence.ledger`), the child-process
scheduler (:mod:`knoted_intelligence.executor`) and the controlled staging tree
(:mod:`knoted_intelligence.staging`).
"""

__all__ = ["__version__"]

__version__ = "0.1.0"
