"""FastAPI application for the internal execution protocol.

The contract is ``api/intelligence.openapi.yaml``; every path below exists
because that file defines it, and the response bodies only carry its fields.
Routes are synchronous on purpose: they do short SQLite work and hand the long
work to the scheduler's child processes, so the event loop is never blocked by a
parse or an embed.

Error bodies follow the shared ``ApiError`` envelope (``code``, ``message``,
``request_id``, optional ``field_errors``), and clients branch on ``code``.
"""

from __future__ import annotations

import logging
import uuid
from collections.abc import Iterator
from contextlib import asynccontextmanager
from datetime import UTC, datetime
from enum import StrEnum
from typing import Any
from uuid import UUID

from fastapi import FastAPI, Request, Response
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict, Field, StrictBool, field_validator, model_validator
from starlette.exceptions import HTTPException as StarletteHTTPException

from . import executor
from .config import Settings
from .executor import ExecutionSpec, Scheduler
from .ledger import ExecutionRow, Ledger
from .staging import StagingError, ensure_object_id
from .supervision import InstanceLock

logger = logging.getLogger(__name__)

#: Widest `source_version` the worker's i64 can hold.
I64_MAX = 2**63 - 1

API_PREFIX = "/internal/v1"
EXECUTIONS_PATH = f"{API_PREFIX}/executions"


class ExecutionKind(StrEnum):
    PARSE = "parse"
    EMBED = "embed"


class ExecutionState(StrEnum):
    QUEUED = "queued"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCEL_REQUESTED = "cancel_requested"
    CANCELLED = "cancelled"
    INTERRUPTED = "interrupted"


class FieldError(BaseModel):
    model_config = ConfigDict(extra="forbid")

    field: str = Field(min_length=1)
    message: str = Field(min_length=1)


class ApiError(BaseModel):
    model_config = ConfigDict(extra="forbid")

    code: str = Field(min_length=1)
    message: str = Field(min_length=1)
    request_id: UUID
    field_errors: list[FieldError] | None = None
    current_version: int | None = None


class ExecutionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    execution_id: UUID
    kind: ExecutionKind
    # The contract pattern uses a lookahead, which Pydantic's regex engine does
    # not support; `_controlled_object_id` below enforces the two extra rules
    # (reject `.`/`..` and any `..` substring) through the shared helper.
    input_object_id: str = Field(pattern=r"^[A-Za-z0-9._-]+$")
    input_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    # The contract says `integer, minimum 0`; the worker stores it in an i64, so
    # anything wider is refused here instead of overflowing later.
    source_version: int = Field(ge=0, le=I64_MAX)
    deadline: datetime
    options_revision: str = Field(min_length=1)

    @field_validator("deadline")
    @classmethod
    def _absolute_deadline(cls, value: datetime) -> datetime:
        # A naive timestamp would silently be read as UTC and could expire a
        # result early or late; the contract format is an absolute instant.
        if value.tzinfo is None or value.utcoffset() is None:
            raise ValueError("deadline must carry a timezone offset")
        return value.astimezone(UTC)

    @field_validator("input_object_id")
    @classmethod
    def _controlled_object_id(cls, value: str) -> str:
        # The pattern already rejects separators and URL schemes; this repeats
        # the containment rule with the same helper every path resolution uses.
        return ensure_object_id(value, field="input_object_id")


class ExecutionRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    execution_id: UUID
    kind: ExecutionKind
    state: ExecutionState
    compute_exit_confirmed: StrictBool = False
    input_sha256: str
    source_version: int
    options_revision: str
    created_at: datetime
    updated_at: datetime
    started_at: datetime | None
    finished_at: datetime | None
    error: ApiError | None

    @model_validator(mode="after")
    def _terminal_proof(self) -> ExecutionRecord:
        if self.compute_exit_confirmed and self.state not in {
            ExecutionState.SUCCEEDED,
            ExecutionState.FAILED,
            ExecutionState.CANCELLED,
            ExecutionState.INTERRUPTED,
        }:
            raise ValueError("compute_exit_confirmed requires a terminal state")
        return self


class ExecutionResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    object_id: str
    sha256: str
    size: int = Field(ge=0)
    media_type: str = Field(min_length=1)
    # The contract types this as `{[k: string]: string}`; it is always emitted
    # as an object so a client never has to special-case a null.
    metadata: dict[str, str] = Field(default_factory=dict)


class HealthState(StrEnum):
    OK = "ok"
    DEGRADED = "degraded"


class HealthStatus(BaseModel):
    model_config = ConfigDict(extra="forbid")

    status: HealthState
    active_executions: int = Field(ge=0)


def create_app(settings: Settings | None = None) -> FastAPI:
    """Builds the application, or the ASGI object uvicorn serves."""

    settings = settings or Settings.from_env()
    ledger = Ledger(settings.ledger_path)
    scheduler = Scheduler(ledger, settings)

    @asynccontextmanager
    async def lifespan(_app: FastAPI) -> Iterator[None]:
        settings.prepare()
        ownership = InstanceLock(settings.state_dir)
        ownership.acquire()
        try:
            ledger.open()
            scheduler.recover()
            try:
                yield
            finally:
                if scheduler.shutdown():
                    ledger.close()
                    ownership.close()
        except BaseException:
            ledger.close()
            ownership.close()
            raise

    app = FastAPI(
        title="Knoted Intelligence Internal API",
        version="1.0.0",
        description="Internal parse and embed executions; see api/intelligence.openapi.yaml.",
        lifespan=lifespan,
    )
    app.state.settings = settings
    app.state.ledger = ledger
    app.state.scheduler = scheduler
    _install_error_handlers(app)

    @app.post(EXECUTIONS_PATH, response_model=ExecutionRecord)
    def create_execution(request: ExecutionRequest, response: Response) -> ExecutionRecord:
        spec = ExecutionSpec(
            execution_id=str(request.execution_id),
            kind=request.kind.value,
            input_object_id=request.input_object_id,
            input_sha256=request.input_sha256,
            source_version=request.source_version,
            options_revision=request.options_revision,
            deadline=request.deadline,
        )
        row, created = scheduler.submit(spec)
        response.status_code = 201 if created else 200
        return _record(row)

    @app.get(f"{EXECUTIONS_PATH}/{{execution_id}}", response_model=ExecutionRecord)
    def get_execution(execution_id: UUID) -> ExecutionRecord:
        return _record(scheduler.record(str(execution_id)))

    @app.post(f"{EXECUTIONS_PATH}/{{execution_id}}/cancel", response_model=ExecutionRecord)
    def cancel_execution(execution_id: UUID) -> ExecutionRecord:
        return _record(scheduler.cancel(str(execution_id)))

    @app.get(f"{EXECUTIONS_PATH}/{{execution_id}}/result", response_model=ExecutionResult)
    def get_execution_result(execution_id: UUID) -> ExecutionResult:
        row = scheduler.result(str(execution_id))
        if row.result_object_id is None or row.result_sha256 is None or row.result_size is None:
            # Succeeded rows always carry an artifact; this is a ledger defect.
            raise executor.ExecutionNotSucceeded(
                f"execution {execution_id} has no registered result"
            )
        return ExecutionResult(
            object_id=row.result_object_id,
            sha256=row.result_sha256,
            size=row.result_size,
            media_type=row.result_media_type or "application/octet-stream",
            metadata=row.result_metadata or {},
        )

    @app.get(f"{API_PREFIX}/health", response_model=HealthStatus)
    def get_health() -> HealthStatus:
        # Degraded means at least one cancellation has not been confirmed: a
        # child is still alive after the supervisor signalled its group.
        degraded, active = scheduler.health()
        return HealthStatus(
            status=HealthState.DEGRADED if degraded else HealthState.OK,
            active_executions=active,
        )

    return app


def _record(row: ExecutionRow) -> ExecutionRecord:
    error = None
    if row.error_code is not None:
        error = ApiError(
            code=row.error_code,
            message=row.error_message or row.error_code,
            request_id=UUID(row.error_request_id) if row.error_request_id else uuid.uuid4(),
        )
    return ExecutionRecord(
        execution_id=UUID(row.execution_id),
        kind=ExecutionKind(row.kind),
        state=ExecutionState(row.state),
        compute_exit_confirmed=row.compute_exit_confirmed,
        input_sha256=row.input_sha256,
        source_version=row.source_version,
        options_revision=row.options_revision,
        created_at=row.created_at,
        updated_at=row.updated_at,
        started_at=row.started_at,
        finished_at=row.finished_at,
        error=error,
    )


def _error_response(
    status_code: int,
    code: str,
    message: str,
    *,
    field_errors: list[FieldError] | None = None,
) -> JSONResponse:
    body = ApiError(
        code=code,
        message=message,
        request_id=uuid.uuid4(),
        field_errors=field_errors,
    )
    return JSONResponse(
        status_code=status_code,
        content=body.model_dump(mode="json", exclude_none=True),
    )


def _install_error_handlers(app: FastAPI) -> None:
    statuses = {
        executor.ExecutionNotFound: 404,
        executor.ExecutionUnavailable: 503,
        executor.ExecutionConflict: 409,
        executor.ExecutionFinished: 409,
        executor.ExecutionNotFinished: 409,
        executor.ExecutionNotSucceeded: 422,
    }

    @app.exception_handler(executor.ExecutionFailure)
    def failure_handler(_request: Request, exc: executor.ExecutionFailure) -> JSONResponse:
        return _error_response(statuses.get(type(exc), 400), exc.code, str(exc))

    @app.exception_handler(RequestValidationError)
    def validation_handler(_request: Request, exc: RequestValidationError) -> JSONResponse:
        field_errors = [
            FieldError(
                field=".".join(str(part) for part in error.get("loc", ()) if part != "body")
                or "body",
                message=str(error.get("msg", "invalid value")),
            )
            for error in exc.errors()
        ]
        return _error_response(
            422,
            "validation_failed",
            "the execution request is invalid",
            field_errors=field_errors,
        )

    @app.exception_handler(StarletteHTTPException)
    def http_exception_handler(_request: Request, exc: StarletteHTTPException) -> JSONResponse:
        codes = {404: "not_found", 405: "method_not_allowed"}
        return _error_response(
            exc.status_code,
            codes.get(exc.status_code, "http_error"),
            str(exc.detail),
        )

    @app.exception_handler(Exception)
    def unexpected_handler(_request: Request, exc: Exception) -> JSONResponse:
        # The log keeps the detail; the body only names the failure class so a
        # response can never leak a filesystem path or an environment value.
        logger.exception("unhandled intelligence service error")
        return _error_response(
            500,
            "internal_error",
            f"the intelligence service failed to handle the request ({type(exc).__name__})",
        )

    @app.exception_handler(StagingError)
    def staging_handler(_request: Request, exc: StagingError) -> JSONResponse:
        return _error_response(400, "invalid_object_id", str(exc))


def main() -> None:
    """Console entry point: ``knoted-intelligence``."""

    import uvicorn

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")
    settings = Settings.from_env()
    app = create_app(settings)
    uvicorn.run(app, host=settings.host, port=settings.port, log_level="info")


def app_from_env() -> Any:  # pragma: no cover - uvicorn --factory target
    return create_app(Settings.from_env())
