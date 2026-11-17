import { queryOptions } from "@tanstack/react-query";
import type { components } from "@knoted/api-client";
import type { TreeItem } from "@bixlab/mtone";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";
import type { Note } from "./autosave";

export function notesTreeQuery(bundle: QueryClientBundle) {
  return queryOptions({
    queryKey: queryKeys.notes.list({ scope: "complete-tree" }), retry: false,
    queryFn: async ({ signal }) => {
      const notes = new Map<string, Note>(); const seen = new Set<string>();
      let cursor: string | null = null;
      do {
        if (cursor !== null) { if (seen.has(cursor)) throw new Error("笔记游标重复，请重试读取完整目录"); seen.add(cursor); }
        const { data }: { data?: components["schemas"]["NotePage"] } = await bundle.api.raw.GET("/notes", { signal, params: { query: { limit: 50, cursor: cursor ?? undefined } } });
        if (!data) throw new Error("无法读取笔记目录");
        for (const note of data.items) notes.set(note.id, note);
        cursor = data.next_cursor;
      } while (cursor !== null);
      const complete = [...notes.values()];
      const reachable = new Set<string>();
      const visit = (items: TreeItem[]) => { for (const item of items) { reachable.add(item.id); visit(item.children ?? []); } };
      visit(noteTree(complete));
      if (reachable.size !== complete.length) throw new Error("目录不完整或父子关系已变化，请重试读取完整目录");
      return complete;
    },
  });
}
export function noteQuery(bundle: QueryClientBundle, noteId: string) {
  return queryOptions({ queryKey: queryKeys.notes.detail(noteId), retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/notes/{noteId}", { signal, params: { path: { noteId } } });
      if (!data) throw new Error("无法读取笔记"); return data;
    },
  });
}
export function noteTree(notes: Note[]): TreeItem[] {
  const groups = new Map<string | null, Note[]>();
  for (const note of notes) { const siblings = groups.get(note.parent_id) ?? []; siblings.push(note); groups.set(note.parent_id, siblings); }
  for (const siblings of groups.values()) siblings.sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));
  const visit = (parent: string | null, ancestors: Set<string>): TreeItem[] => (groups.get(parent) ?? []).filter(note => !ancestors.has(note.id)).map(note => ({ id: note.id, label: note.title, children: visit(note.id, new Set([...ancestors, note.id])) }));
  return visit(null, new Set());
}
export function descendants(notes: Note[], id: string): Set<string> {
  const result = new Set([id]); let changed = true;
  while (changed) { changed = false; for (const note of notes) if (note.parent_id !== null && result.has(note.parent_id) && !result.has(note.id)) { result.add(note.id); changed = true; } }
  return result;
}
