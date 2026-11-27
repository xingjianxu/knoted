import { queryOptions } from "@tanstack/react-query";
import type { components, operations } from "@knoted/api-client";
import { z } from "zod";
import { queryKeys, type QueryClientBundle } from "../../lib/query";

export type SearchHit = components["schemas"]["SearchHit"];
export type SearchFilters = Omit<operations["search"]["parameters"]["query"], "cursor" | "limit">;

export const searchTypes = {
  note: "笔记",
  task: "事项",
  project: "项目",
} satisfies Record<components["schemas"]["SearchType"], string>;

export const searchSorts = {
  relevance: "相关性",
  updated_desc: "更新时间从新到旧",
  updated_asc: "更新时间从旧到新",
} satisfies Record<components["schemas"]["SearchSort"], string>;

const entityId = z.uuid();
const timestamp = z.iso.datetime({ offset: true });

function text(value: unknown): string | undefined {
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function validateSearch(search: Record<string, unknown>): SearchFilters {
  const tags = Array.isArray(search.tags) ? search.tags : [search.tags];
  const validTags = [...new Set(tags.flatMap((tag) => {
    const id = entityId.safeParse(tag).data;
    return id ? [id.toLowerCase()] : [];
  }))].sort();
  return {
    q: Array.from(text(search.q) ?? "").slice(0, 500).join(""),
    type: z.enum(searchTypesKeys).safeParse(search.type).data,
    tags: validTags.length ? validTags : undefined,
    project: entityId.safeParse(search.project).data?.toLowerCase(),
    updated_from: timestamp.safeParse(search.updated_from).data,
    updated_to: timestamp.safeParse(search.updated_to).data,
    archived: typeof search.archived === "boolean" ? search.archived : undefined,
    sort: z.enum(searchSortKeys).safeParse(search.sort).data,
  };
}

const searchTypesKeys = ["note", "task", "project"] as const;
const searchSortKeys = ["relevance", "updated_desc", "updated_asc"] as const;

export function searchQuery(bundle: QueryClientBundle, filters: SearchFilters, cursor: string | null) {
  const query: operations["search"]["parameters"]["query"] = {
    ...filters,
    sort: filters.sort ?? "relevance",
    limit: 50,
    cursor: cursor ?? undefined,
  };
  return queryOptions({
    queryKey: queryKeys.search(query),
    enabled: filters.q.length > 0,
    staleTime: 0,
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/search", { params: { query }, signal });
      if (!data) throw new Error("无法读取搜索结果");
      return data;
    },
  });
}

export function searchTagChoicesQuery(bundle: QueryClientBundle) {
  return queryOptions({
    queryKey: queryKeys.tags.list({ purpose: "search-choices" }),
    retry: false,
    queryFn: async ({ signal }) => {
      const tags: components["schemas"]["Tag"][] = [];
      const seen = new Set<string>();
      let cursor: string | undefined;
      do {
        const { data } = await bundle.api.raw.GET("/tags", { signal, params: { query: { limit: 100, cursor } } });
        if (!data) throw new Error("无法读取标签");
        tags.push(...data.items);
        cursor = data.next_cursor ?? undefined;
        if (cursor && seen.has(cursor)) throw new Error("标签游标重复");
        if (cursor) seen.add(cursor);
      } while (cursor);
      return tags;
    },
  });
}
