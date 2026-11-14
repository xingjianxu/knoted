import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { ApiRequestError, createApiClient } from "@knoted/api-client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createQueryClient, normalizeFilters, queryKeys } from "../../src/lib/query";

// Node's Request rejects relative URLs; browsers resolve them against the page
// origin. The app uses baseUrl "/api/v1", so tests need the browser behaviour.
class BrowserRequest extends Request {
  constructor(input: RequestInfo | URL, init?: RequestInit) {
    super(typeof input === "string" ? new URL(input, window.location.origin) : input, init);
  }
}

const testDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(testDir, "../../../..");
const schemaPath = path.join(repoRoot, "packages/api-client/src/generated/schema.d.ts");

function sha256(filePath: string): string {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex");
}

function cookieRequest(fake: typeof fetch): Request {
  const calls = vi.mocked(fake).mock.calls;
  const input = calls.at(-1)?.[0];
  if (!(input instanceof Request)) {
    throw new Error("api client did not call fetch with a Request");
  }
  return input;
}

function clearCsrfCookie(): void {
  document.cookie = "knoted_csrf=; Max-Age=0; path=/";
}

beforeEach(() => {
  vi.stubGlobal("Request", BrowserRequest);
});

afterEach(() => {
  vi.unstubAllGlobals();
  clearCsrfCookie();
  vi.restoreAllMocks();
});

describe("t04 generated client", () => {
  it("t04_1_generation_is_deterministic", () => {
    execFileSync("pnpm", ["run", "api:generate"], { cwd: repoRoot, stdio: "pipe" });
    const first = sha256(schemaPath);
    execFileSync("pnpm", ["run", "api:generate"], { cwd: repoRoot, stdio: "pipe" });
    const second = sha256(schemaPath);

    expect(second).toBe(first);
    expect(readFileSync(schemaPath, "utf8")).toContain("export interface paths");
  });

  it("t04_1_query_keys_normalize_filters", () => {
    expect(
      queryKeys.notes.list({ parent_id: "x", limit: 50 }),
    ).toEqual(queryKeys.notes.list({ limit: 50, parent_id: "x" }));
    expect(
      queryKeys.notes.list({ parent_id: "x", limit: undefined }),
    ).toEqual(queryKeys.notes.list({ parent_id: "x", limit: "" }));
    expect(queryKeys.notes.list({ parent_id: "x" })).not.toEqual(
      queryKeys.notes.list({ parent_id: "y" }),
    );
    expect(queryKeys.notes.list({})).not.toEqual(queryKeys.notes.detail("id"));
    expect(normalizeFilters({ b: 1, a: 2, empty: "", missing: undefined })).toEqual({
      a: 2,
      b: 1,
    });
  });

  it("t04_1_cancel_passes_abort_signal", async () => {
    let capturedSignal: AbortSignal | undefined;
    const fakeFetch = vi.fn<typeof fetch>((input) => {
      const request = input instanceof Request ? input : new Request(input);
      capturedSignal = request.signal;
      return new Promise<Response>((_resolve, reject) => {
        request.signal.addEventListener("abort", () => {
          reject(new DOMException("The operation was aborted.", "AbortError"));
        });
      });
    });
    vi.stubGlobal("fetch", fakeFetch);

    const { queryClient, api } = createQueryClient();
    const pending = queryClient.fetchQuery({
      queryKey: queryKeys.notes.list({}),
      queryFn: ({ signal: querySignal }) =>
        api.raw.GET("/notes", { signal: querySignal }),
      retry: false,
    });

    await vi.waitFor(() => expect(fakeFetch).toHaveBeenCalledTimes(1));
    await queryClient.cancelQueries({ queryKey: queryKeys.notes.list({}) });

    await expect(pending).rejects.toThrow();
    expect(capturedSignal?.aborted).toBe(true);

    queryClient.clear();
  });

  it("t04_1_forwards_abort_signal", async () => {
    let capturedSignal: AbortSignal | undefined;
    const fakeFetch = vi.fn<typeof fetch>((input) => {
      const request = input instanceof Request ? input : new Request(input);
      capturedSignal = request.signal;
      return new Promise<Response>((_resolve, reject) => {
        request.signal.addEventListener("abort", () => {
          reject(new DOMException("The operation was aborted.", "AbortError"));
        });
      });
    });
    vi.stubGlobal("fetch", fakeFetch);
    const { raw } = createApiClient({ baseUrl: "/api/v1", fetch: fakeFetch });
    const controller = new AbortController();

    const pending = raw.GET("/notes", { signal: controller.signal });
    await vi.waitFor(() => expect(fakeFetch).toHaveBeenCalledTimes(1));
    controller.abort();

    expect(capturedSignal?.aborted).toBe(true);
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  });

  it("t04_1_unauthorized_clears_cache_without_retry", async () => {
    const fetchMock = vi.fn<typeof fetch>(
      async () =>
        new Response(
          JSON.stringify({
            code: "unauthorized",
            message: "no session",
            request_id: "00000000-0000-0000-0000-000000000000",
          }),
          { status: 401, headers: { "Content-Type": "application/json" } },
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const onUnauthorized = vi.fn();
    const { queryClient, api } = createQueryClient({ onUnauthorized });
    const seededKey = queryKeys.notes.list({});
    queryClient.setQueryData(seededKey, { items: [], next_cursor: null });

    const pending = queryClient.fetchQuery({
      queryKey: queryKeys.tasks.list({ status: "open" }),
      queryFn: ({ signal }) => api.raw.GET("/tasks", { signal }),
    });

    await expect(pending).rejects.toBeInstanceOf(ApiRequestError);
    await expect(pending).rejects.toMatchObject({
      status: 401,
      code: "unauthorized",
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => {
      expect(queryClient.getQueryData(seededKey)).toBeUndefined();
      expect(queryClient.getQueryCache().getAll()).toHaveLength(0);
    });

    queryClient.clear();
  });

  it("t04_1_error_envelope_and_fallback", async () => {
    let nextResponse = new Response("upstream unavailable", {
      status: 500,
      headers: { "Content-Type": "text/plain" },
    });
    const fakeFetch = vi.fn<typeof fetch>(async () => nextResponse);
    vi.stubGlobal("fetch", fakeFetch);
    const { raw } = createApiClient({ baseUrl: "/api/v1", fetch: fakeFetch });

    await expect(raw.GET("/notes")).rejects.toMatchObject({
      status: 500,
      code: "http_500",
      requestId: "",
      message: "HTTP 500",
    });

    nextResponse = new Response(
      JSON.stringify({
        code: "validation_failed",
        message: "invalid note",
        request_id: "00000000-0000-0000-0000-000000000002",
        field_errors: [
          { field: "title", message: "required" },
          { field: 1, message: "ignored" },
        ],
        current_version: 7,
      }),
      { status: 422, headers: { "Content-Type": "application/json" } },
    );

    await expect(
      raw.PATCH("/notes/{noteId}", {
        params: { path: { noteId: "00000000-0000-0000-0000-000000000003" } },
        body: { expected_version: 6 },
      }),
    ).rejects.toMatchObject({
      status: 422,
      code: "validation_failed",
      requestId: "00000000-0000-0000-0000-000000000002",
      fieldErrors: [{ field: "title", message: "required" }],
      currentVersion: 7,
    });
  });

  it("t04_1_401_clear_is_scoped", async () => {
    const fetchMock = vi.fn<typeof fetch>(
      async () =>
        new Response(
          JSON.stringify({
            code: "unauthorized",
            message: "no session",
            request_id: "00000000-0000-0000-0000-000000000000",
          }),
          { status: 401, headers: { "Content-Type": "application/json" } },
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const onUnauthorized = vi.fn();
    const { queryClient, api } = createQueryClient({ onUnauthorized });
    const beforeKey = queryKeys.notes.detail("before-401");
    const afterKey = queryKeys.notes.detail("after-401");
    queryClient.setQueryData(beforeKey, { id: "before-401" });

    await expect(
      queryClient.fetchQuery({
        queryKey: queryKeys.tasks.list({}),
        queryFn: ({ signal }) => api.raw.GET("/tasks", { signal }),
      }),
    ).rejects.toBeInstanceOf(ApiRequestError);

    // Cached after the 401 response but before the deferred removal runs.
    queryClient.setQueryData(afterKey, { id: "after-401" });

    await vi.waitFor(() => {
      expect(queryClient.getQueryData(beforeKey)).toBeUndefined();
    });
    expect(queryClient.getQueryData(afterKey)).toEqual({ id: "after-401" });
    expect(onUnauthorized).toHaveBeenCalledTimes(1);

    queryClient.clear();
  });

  it("t04_1_sends_credentials_and_csrf", async () => {
    document.cookie = "knoted_csrf=token-1";
    const noteBody = JSON.stringify({
      id: "00000000-0000-0000-0000-000000000001",
      parent_id: null,
      title: "T",
      document: { blocks: [] },
      format_version: 1,
      content_version: 1,
      position: 0,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      deleted_at: null,
    });
    const fakeFetch = vi.fn<typeof fetch>(async (input) => {
      const method = input instanceof Request ? input.method : "GET";
      return new Response(
        method === "GET" ? JSON.stringify({ items: [], next_cursor: null }) : noteBody,
        {
          status: method === "GET" ? 200 : 201,
          headers: { "Content-Type": "application/json" },
        },
      );
    });
    vi.stubGlobal("fetch", fakeFetch);
    const { raw } = createApiClient({ baseUrl: "/api/v1", fetch: fakeFetch });

    await raw.POST("/notes", {
      body: { title: "T", document: { blocks: [{ id: "b1", type: "paragraph" }] } },
    });

    const postRequest = cookieRequest(fakeFetch);
    expect(postRequest.credentials).toBe("same-origin");
    expect(postRequest.headers.get("X-CSRF-Token")).toBe("token-1");
    expect(postRequest.url).toBe(`${window.location.origin}/api/v1/notes`);

    await raw.GET("/notes");
    const getRequest = cookieRequest(fakeFetch);
    expect(getRequest.method).toBe("GET");
    expect(getRequest.credentials).toBe("same-origin");
    expect(getRequest.headers.get("X-CSRF-Token")).toBeNull();

    clearCsrfCookie();
    await raw.POST("/notes", {
      body: { title: "T2", document: { blocks: [{ id: "b2", type: "paragraph" }] } },
    });

    expect(cookieRequest(fakeFetch).headers.get("X-CSRF-Token")).toBeNull();
  });
});
