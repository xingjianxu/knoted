import createClient, { type Client } from "openapi-fetch";

import { ApiRequestError } from "./errors";
import type { paths } from "./generated/schema";

export interface ApiClientOptions {
  /** Prefix for every request, e.g. "/api/v1". */
  baseUrl: string;
  /** Injectable fetch for tests; defaults to the global fetch at call time. */
  fetch?: typeof fetch;
  /** Invoked at most once per 401 response. Routing/login belong to the caller. */
  onUnauthorized?: () => void;
  /** Cookie carrying the CSRF token; defaults to "knoted_csrf". */
  csrfCookieName?: string;
}

export interface ApiClient {
  /** Typed openapi-fetch client generated from api/openapi.yaml. */
  raw: Client<paths>;
}

const DEFAULT_CSRF_COOKIE_NAME = "knoted_csrf";
const SAFE_METHODS = new Set(["GET", "HEAD"]);

function readCookie(name: string): string | undefined {
  if (typeof document === "undefined") {
    return undefined;
  }

  const prefix = `${name}=`;
  for (const part of document.cookie.split(";")) {
    const entry = part.trim();
    if (!entry.startsWith(prefix)) {
      continue;
    }

    const value = entry.slice(prefix.length);
    try {
      return decodeURIComponent(value);
    } catch {
      return value;
    }
  }

  return undefined;
}

export function createApiClient(options: ApiClientOptions): ApiClient {
  const fetchImpl = options.fetch ?? ((input, init) => globalThis.fetch(input, init));
  const csrfCookieName = options.csrfCookieName ?? DEFAULT_CSRF_COOKIE_NAME;

  const raw = createClient<paths>({
    baseUrl: options.baseUrl,
    credentials: "same-origin",
    fetch: fetchImpl,
  });

  raw.use({
    onRequest({ request }) {
      if (SAFE_METHODS.has(request.method.toUpperCase())) {
        return request;
      }

      const csrfToken = readCookie(csrfCookieName);
      if (csrfToken) {
        request.headers.set("X-CSRF-Token", csrfToken);
      }

      return request;
    },
    async onResponse({ response }) {
      if (response.ok) {
        return;
      }

      const error = await ApiRequestError.fromResponse(response);
      if (response.status === 401) {
        // Cache eviction runs in the next task. Finish reading the body first
        // so the typed rejection propagates through promises before eviction.
        options.onUnauthorized?.();
      }

      throw error;
    },
  });

  return { raw };
}
