"""Runtime configuration for the internal intelligence service.

There is one source of configuration: the environment. Every variable has a
default that matches the deployment layout, so the service starts with no
configuration at all. Tests construct :class:`Settings` directly with a
temporary directory; nothing in the repository hard-codes a writable path.

The service is stateful in exactly two places, both configured here:

* the SQLite execution ledger (``state_dir``), which stores execution state and
  nothing else — the business database stays in PostgreSQL;
* the staging tree (``staging_dir``) with the controlled ``input`` and
  ``output`` roots that object ids map into.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

#: Directory holding the SQLite ledger.
DEFAULT_STATE_DIR = Path("/data/knoted-dev/service/state/intelligence")
#: Root of the controlled staging tree.
DEFAULT_STAGING_DIR = Path("/data/knoted-dev/service/staging")
#: Executions allowed to run at the same time (one GPU card, two slots).
DEFAULT_MAX_SLOTS = 2
#: Parse and embed executions share one GPU slot by default: OCR and embeddings
#: use the same card and must not resident both models at once.
DEFAULT_GPU_SLOTS = 1
#: Loopback only: the service is never reachable from outside the worker host.
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 15603
#: How long a child may take to honour SIGTERM before its group gets SIGKILL.
DEFAULT_CANCEL_GRACE_SECONDS = 5.0

#: Ledger file name inside :attr:`Settings.state_dir`.
LEDGER_FILENAME = "executions.db"
#: Sub-directory of :attr:`Settings.staging_dir` holding execution inputs.
INPUT_DIRNAME = "input"
#: Sub-directory of :attr:`Settings.staging_dir` holding execution artifacts.
OUTPUT_DIRNAME = "output"

STATE_DIR_ENV = "KNOTED_INTELLIGENCE_STATE_DIR"
STAGING_DIR_ENV = "KNOTED_INTELLIGENCE_STAGING_DIR"
MAX_SLOTS_ENV = "KNOTED_INTELLIGENCE_MAX_SLOTS"
GPU_SLOTS_ENV = "KNOTED_INTELLIGENCE_GPU_SLOTS"
HOST_ENV = "KNOTED_INTELLIGENCE_HOST"
PORT_ENV = "KNOTED_INTELLIGENCE_PORT"
CHILD_HOLD_MS_ENV = "KNOTED_INTELLIGENCE_CHILD_HOLD_MS"
CANCEL_GRACE_SECONDS_ENV = "KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS"
CHILD_IGNORE_SIGTERM_ENV = "KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM"


@dataclass(frozen=True, slots=True)
class Settings:
    """Everything the service needs to know about its surroundings."""

    state_dir: Path = DEFAULT_STATE_DIR
    staging_dir: Path = DEFAULT_STAGING_DIR
    max_slots: int = DEFAULT_MAX_SLOTS
    #: Slots for GPU-class work (parse + embed). Both kinds acquire this before
    #: their child starts, so OCR and embeddings never resident together.
    gpu_slots: int = DEFAULT_GPU_SLOTS
    host: str = DEFAULT_HOST
    port: int = DEFAULT_PORT
    #: How long a cancellation waits for a child to honour SIGTERM before the
    #: process group is killed outright.
    cancel_grace_seconds: float = DEFAULT_CANCEL_GRACE_SECONDS
    #: Test hook: milliseconds the child waits before it computes.
    #: It does not change the state machine, only how long a real child process
    #: stays alive, which is what cancellation tests need to observe.
    child_hold_ms: int = 0
    #: Test hook: makes the child ignore SIGTERM, so the supervisor's
    #: SIGKILL escalation is exercised against a genuinely unresponsive child.
    child_ignore_sigterm: bool = False

    def __post_init__(self) -> None:
        if self.max_slots < 1:
            raise ValueError("max_slots must be at least 1")
        if self.gpu_slots < 1:
            raise ValueError("gpu_slots must be at least 1")
        if self.gpu_slots > self.max_slots:
            raise ValueError("gpu_slots must not exceed max_slots")
        if not 1 <= self.port <= 65535:
            raise ValueError(f"port {self.port} is outside 1..=65535")
        if self.child_hold_ms < 0:
            raise ValueError("child_hold_ms must not be negative")
        if self.cancel_grace_seconds <= 0:
            raise ValueError("cancel_grace_seconds must be positive")

    @classmethod
    def from_env(cls) -> Settings:
        """Reads the environment, falling back to the deployment defaults."""

        return cls(
            state_dir=_path_env(STATE_DIR_ENV, DEFAULT_STATE_DIR),
            staging_dir=_path_env(STAGING_DIR_ENV, DEFAULT_STAGING_DIR),
            max_slots=_int_env(MAX_SLOTS_ENV, DEFAULT_MAX_SLOTS),
            gpu_slots=_int_env(GPU_SLOTS_ENV, DEFAULT_GPU_SLOTS),
            host=_str_env(HOST_ENV, DEFAULT_HOST),
            port=_int_env(PORT_ENV, DEFAULT_PORT),
            cancel_grace_seconds=_float_env(CANCEL_GRACE_SECONDS_ENV, DEFAULT_CANCEL_GRACE_SECONDS),
            child_hold_ms=_int_env(CHILD_HOLD_MS_ENV, 0),
            child_ignore_sigterm=_bool_env(CHILD_IGNORE_SIGTERM_ENV, False),
        )

    @property
    def ledger_path(self) -> Path:
        return self.state_dir / LEDGER_FILENAME

    @property
    def input_dir(self) -> Path:
        return self.staging_dir / INPUT_DIRNAME

    @property
    def output_dir(self) -> Path:
        return self.staging_dir / OUTPUT_DIRNAME

    def prepare(self) -> None:
        """Creates the state and staging directories if they are missing."""

        for directory in (self.state_dir, self.staging_dir, self.input_dir, self.output_dir):
            directory.mkdir(parents=True, exist_ok=True)


def _str_env(name: str, default: str) -> str:
    value = os.environ.get(name)
    return value if value and value.strip() else default


def _path_env(name: str, default: Path) -> Path:
    return Path(_str_env(name, str(default))).expanduser()


def _float_env(name: str, default: float) -> float:
    raw = os.environ.get(name)
    if raw is None or not raw.strip():
        return default
    try:
        return float(raw)
    except ValueError as exc:  # pragma: no cover - operator error
        raise ValueError(f"{name} must be a number, got {raw!r}") from exc


def _bool_env(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None or not raw.strip():
        return default
    return raw.strip().lower() in {"1", "true", "yes", "on"}


def _int_env(name: str, default: int) -> int:
    raw = os.environ.get(name)
    if raw is None or not raw.strip():
        return default
    try:
        return int(raw)
    except ValueError as exc:  # pragma: no cover - operator error
        raise ValueError(f"{name} must be an integer, got {raw!r}") from exc
