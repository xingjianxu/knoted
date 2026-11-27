export interface ApiFieldError {
  readonly field: string;
  readonly message: string;
}

export interface ApiRequestErrorInit {
  status: number;
  code: string;
  requestId: string;
  message?: string;
  fieldErrors?: readonly ApiFieldError[];
  currentVersion?: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

// Error envelopes contain diagnostics, including per-field validation messages.
const MAX_ERROR_BODY_BYTES = 64 * 1024;

async function readErrorBody(response: Response, signal?: AbortSignal): Promise<unknown> {
  const reader = response.body?.getReader();
  if (!reader) return;

  // A stream's source can take arbitrarily long to acknowledge cancellation.
  // The HTTP error is already known; only initiate cancellation before returning.
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    if (signal?.aborted) {
      cancel();
      return;
    }

    const decoder = new TextDecoder();
    let bytes = 0;
    let text = "";
    while (true) {
      const { done, value } = await reader.read();
      if (signal?.aborted) return;
      if (done) return JSON.parse(text + decoder.decode());

      bytes += value.byteLength;
      if (bytes > MAX_ERROR_BODY_BYTES) {
        cancel();
        return;
      }
      text += decoder.decode(value, { stream: true });
    }
  } finally {
    signal?.removeEventListener("abort", cancel);
    reader.releaseLock();
  }
}

/**
 * Error raised for every non-2xx API response. The public shape mirrors the
 * ApiError envelope (api/openapi.yaml) without leaking the raw response.
 */
export class ApiRequestError extends Error {
  readonly status: number;
  readonly code: string;
  readonly requestId: string;
  readonly fieldErrors?: readonly ApiFieldError[];
  readonly currentVersion?: number;

  constructor(init: ApiRequestErrorInit) {
    super(init.message ?? `HTTP ${init.status}`);
    this.name = "ApiRequestError";
    this.status = init.status;
    this.code = init.code;
    this.requestId = init.requestId;
    this.fieldErrors = init.fieldErrors;
    this.currentVersion = init.currentVersion;
  }

  static async fromResponse(response: Response, signal?: AbortSignal): Promise<ApiRequestError> {
    const fallback = {
      status: response.status,
      code: `http_${response.status}`,
      requestId: response.headers.get("X-Request-Id") ?? "",
      message: `HTTP ${response.status}`,
    } satisfies ApiRequestErrorInit;

    let body: unknown;
    try {
      body = await readErrorBody(response, signal);
    } catch {
      return new ApiRequestError(fallback);
    }

    if (!isRecord(body) || !isNonEmptyString(body.code) || !isNonEmptyString(body.message)) {
      return new ApiRequestError(fallback);
    }

    const fieldErrors = Array.isArray(body.field_errors)
      ? body.field_errors.flatMap((entry) => {
          if (!isRecord(entry) || !isNonEmptyString(entry.field) || !isNonEmptyString(entry.message)) {
            return [];
          }
          return [{ field: entry.field, message: entry.message }];
        })
      : undefined;

    return new ApiRequestError({
      status: response.status,
      code: body.code,
      message: body.message,
      requestId: isNonEmptyString(body.request_id) ? body.request_id : fallback.requestId,
      fieldErrors: fieldErrors && fieldErrors.length > 0 ? fieldErrors : undefined,
      currentVersion:
        typeof body.current_version === "number" ? body.current_version : undefined,
    });
  }
}
