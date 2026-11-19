"""Serve the frozen artifact's ASGI factory on the private proxy socket."""
import logging
import os
from pathlib import Path
import sys
import uvicorn
from knoted_intelligence.app import app_from_env

logging.basicConfig(level=logging.INFO)
endpoint = Path(sys.argv[1])
# Only this service writes the private parent directory. A SIGKILL may have
# left the previous server's socket inode behind.
endpoint.unlink(missing_ok=True)
uvicorn.run(app_from_env(), uds=os.fspath(endpoint), log_level="info")
