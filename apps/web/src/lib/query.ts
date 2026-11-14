import { QueryClient } from "@tanstack/react-query";

import { ApiRequestError, createApiClient, type ApiClient } from "@knoted/api-client";

export type QueryFilters = Record<string, unknown>;

/**
 * Drops empty filters (undefined / "") and sorts keys so that two callers who
 * describe the same list query build a byte-identical cache key. Without this,
 * route prefetch and in-page fetches would never share a cache entry.
 */
export function normalizeFilters(filters: QueryFilters | undefined): QueryFilters {
  if (!filters) {
    return {};
  }

  const normalized: QueryFilters = {};
  for (const key of Object.keys(filters).sort()) {
    const value = filters[key];
    if (value === undefined || value === "") {
      continue;
    }
    normalized[key] = value;
  }
  return normalized;
}

export const queryKeys = {
  auth: {
    session: () => ["auth", "session"] as const,
    sessions: () => ["auth", "sessions"] as const,
    passkeys: () => ["auth", "passkeys"] as const,
  },
  notes: {
    all: () => ["notes"] as const,
    list: (filters?: QueryFilters) => ["notes", "list", normalizeFilters(filters)] as const,
    detail: (id: string) => ["notes", "detail", id] as const,
    snapshots: (id: string) => ["notes", "detail", id, "snapshots"] as const,
  },
  tasks: {
    all: () => ["tasks"] as const,
    list: (filters?: QueryFilters) => ["tasks", "list", normalizeFilters(filters)] as const,
    detail: (id: string) => ["tasks", "detail", id] as const,
    activities: (id: string) => ["tasks", "detail", id, "activities"] as const,
  },
  projects: {
    all: () => ["projects"] as const,
    list: (filters?: QueryFilters) => ["projects", "list", normalizeFilters(filters)] as const,
    detail: (id: string) => ["projects", "detail", id] as const,
  },
  tags: {
    all: () => ["tags"] as const,
    list: (filters?: QueryFilters) => ["tags", "list", normalizeFilters(filters)] as const,
  },
  files: {
    all: () => ["files"] as const,
    list: (filters?: QueryFilters) => ["files", "list", normalizeFilters(filters)] as const,
    detail: (id: string) => ["files", "detail", id] as const,
  },
  search: (filters?: QueryFilters) => ["search", normalizeFilters(filters)] as const,
  jobs: {
    all: () => ["jobs"] as const,
    list: (filters?: QueryFilters) => ["jobs", "list", normalizeFilters(filters)] as const,
  },
  dashboard: () => ["dashboard"] as const,
  trash: () => ["trash"] as const,
  chat: {
    sessions: () => ["chat", "sessions"] as const,
    session: (id: string) => ["chat", "sessions", id] as const,
    messages: (id: string) => ["chat", "sessions", id, "messages"] as const,
  },
} as const;

const NO_RETRY_STATUSES = new Set([401, 403, 404, 422]);

export interface QueryClientBundle {
  queryClient: QueryClient;
  api: ApiClient;
}

/**
 * Creates the one QueryClient + ApiClient pair for the app. Callers create it
 * once (T20) and share it; there is deliberately no module-level singleton.
 *
 * Cache policy: server data is stale after 30s, focus refetches are disabled
 * to avoid surprise draft clobbering, and authentication/validation failures
 * are never retried (the 401 path also clears the whole cache).
 */
export function createQueryClient(options?: { onUnauthorized?: () => void }): QueryClientBundle {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: (failureCount, error) => {
          if (error instanceof ApiRequestError && NO_RETRY_STATUSES.has(error.status)) {
            return false;
          }
          return failureCount < 2;
        },
        staleTime: 30_000,
        refetchOnWindowFocus: false,
      },
    },
  });

  const api = createApiClient({
    baseUrl: "/api/v1",
    onUnauthorized: () => {
      // clear() also cancels in-flight queries. Deferring one task keeps the
      // triggering 401 reaching its caller as ApiRequestError (not TanStack's
      // CancelledError) while still dropping every cached entry before paint.
      setTimeout(() => queryClient.clear(), 0);
      options?.onUnauthorized?.();
    },
  });

  return { queryClient, api };
}
