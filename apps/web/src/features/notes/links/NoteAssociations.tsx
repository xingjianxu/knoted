import { useReducer, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, Button, Empty, FormField, Input, List, ListItem, Select, Spinner } from "@bixlab/mtone";
import type { ApiClient, components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../../lib/cursor-state";

type EntityType = components["schemas"]["EntityEndType"];
export interface NoteAssociationsProps {
  noteId: string;
  api: ApiClient;
  readOnly?: boolean;
  /** Fresh workspace policy; readOnly alone cannot stop an awaited continuation. */
  assertCanWrite?: () => void;
}
/** Business composition; the notes page owns selection and mounts this beside the editor. */
export function NoteAssociations(props: NoteAssociationsProps) {
  return <NoteAssociationsContent key={props.noteId} {...props} />;
}
function NoteAssociationsContent({ noteId, api, readOnly = false, assertCanWrite = () => {} }: NoteAssociationsProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [targetType, setTargetType] = useState<EntityType>("note");
  const [targetId, setTargetId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [tagPage, tagDispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const [linkPage, linkDispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const [optionsPage, optionsDispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const root = ["note-associations", noteId];
  const tags = useQuery({
    queryKey: [...root, "tags", tagPage.cursor], retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await api.raw.GET("/entities/{entityType}/{entityId}/tags", {
        signal, params: { path: { entityType: "note", entityId: noteId }, query: { limit: 50, cursor: tagPage.cursor ?? undefined } },
      });
      if (!data) throw new Error("无法读取标签"); return data;
    },
  });
  const options = useQuery({
    queryKey: ["tag-options", optionsPage.cursor], retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await api.raw.GET("/tags", { signal, params: { query: { limit: 50, cursor: optionsPage.cursor ?? undefined } } });
      if (!data) throw new Error("无法读取已有标签"); return data;
    },
  });
  const links = useQuery({
    queryKey: [...root, "links", linkPage.cursor], retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await api.raw.GET("/links", { signal, params: { query: { source_type: "note", source_id: noteId, limit: 50, cursor: linkPage.cursor ?? undefined } } });
      if (!data) throw new Error("无法读取资料关联"); return data;
    },
  });
  const loading = tags.isFetching || links.isFetching || options.isFetching;
  const disabled = readOnly || busy || loading || tags.isError || links.isError || options.isError;
  async function mutate(operation: () => Promise<void>) {
    if (disabled) return;
    setBusy(true); setError("");
    try {
      await operation();
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: root }),
        queryClient.invalidateQueries({ queryKey: ["tag-options"] }),
      ]);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "操作失败，请重试"); }
    finally { setBusy(false); }
  }
  async function assign(tagId: string) {
    assertCanWrite();
    await api.raw.PUT("/entities/{entityType}/{entityId}/tags/{tagId}", { params: { path: { entityType: "note", entityId: noteId, tagId } } });
  }
  const queryError = tags.error ?? links.error ?? options.error;
  return <section aria-label="笔记标签与资料关联" className="space-y-4">
    <h2>标签与资料关联</h2>
    {loading && <Spinner label="正在读取关联" />}
    {(error || queryError) && <Alert tone="error">{error || (queryError instanceof Error ? queryError.message : "读取失败")}</Alert>}
    {queryError && <Button onClick={() => { void tags.refetch(); void links.refetch(); void options.refetch(); }} disabled={loading}>重试读取</Button>}
    <form onSubmit={event => { event.preventDefault(); void mutate(async () => {
      if (!name.trim()) throw new Error("请输入标签名称");
      assertCanWrite();
      const { data } = await api.raw.POST("/tags", { body: { name: name.trim() } });
      if (!data) throw new Error("无法创建标签");
      // Keep the created ID selected if assignment fails; retry does not create it twice.
      setSelectedTag(data.id); setName(""); await assign(data.id);
    }); }} className="flex flex-wrap items-end gap-2">
      <FormField label="新标签名称"><Input value={name} maxLength={64} onChange={e => setName(e.target.value)} disabled={disabled} /></FormField>
      <Button type="submit" disabled={disabled}>创建并添加标签</Button>
    </form>
    <div className="flex flex-wrap items-end gap-2">
      <FormField label="已有标签"><Select value={selectedTag} onValueChange={setSelectedTag} options={(options.data?.items ?? []).map(tag => ({ value: tag.id, label: tag.name }))} disabled={disabled} /></FormField>
      <Button disabled={disabled || !selectedTag} onClick={() => void mutate(async () => { if (selectedTag) await assign(selectedTag); })}>添加已有标签</Button>
      <Button disabled={disabled || optionsPage.previous.length === 0} onClick={() => optionsDispatch({ type: "previous" })}>上一页已有标签</Button>
      <Button disabled={disabled || !options.data?.next_cursor} onClick={() => optionsDispatch({ type: "next", nextCursor: options.data?.next_cursor ?? null })}>下一页已有标签</Button>
    </div>
    <List aria-label="笔记标签">
      {tags.data?.items.map(tag => <ListItem key={tag.id} actions={<Button disabled={disabled} aria-label={`移除标签 ${tag.name}`} onClick={() => void mutate(async () => {
        assertCanWrite();
        await api.raw.DELETE("/entities/{entityType}/{entityId}/tags/{tagId}", { params: { path: { entityType: "note", entityId: noteId, tagId: tag.id } } });
      })}>移除</Button>}>{tag.name}</ListItem>)}
    </List>
    {tags.data?.items.length === 0 && <Empty title="暂无标签" />}
    <div className="flex gap-2">
      <Button disabled={disabled || tagPage.previous.length === 0} onClick={() => tagDispatch({ type: "previous" })}>上一页标签</Button>
      <Button disabled={disabled || !tags.data?.next_cursor} onClick={() => tagDispatch({ type: "next", nextCursor: tags.data?.next_cursor ?? null })}>下一页标签</Button>
    </div>
    <form onSubmit={event => { event.preventDefault(); void mutate(async () => {
      if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(targetId.trim())) throw new Error("请输入有效的目标 ID");
      assertCanWrite();
      await api.raw.POST("/links", { body: { source_type: "note", source_id: noteId, target_type: targetType, target_id: targetId.trim() } }); setTargetId("");
    }); }} className="flex flex-wrap items-end gap-2">
      <FormField label="关联目标类型"><Select value={targetType} onValueChange={value => { if (value) setTargetType(value as EntityType); }} options={[{ value: "note", label: "笔记" }, { value: "file", label: "文件" }, { value: "project", label: "项目" }, { value: "task", label: "事项" }]} disabled={disabled} /></FormField>
      <FormField label="关联目标 ID"><Input value={targetId} onChange={e => setTargetId(e.target.value)} disabled={disabled} /></FormField>
      <Button type="submit" disabled={disabled}>添加关联</Button>
    </form>
    <List aria-label="资料关联">
      {links.data?.items.map(link => <ListItem key={link.id} description={link.target_type} actions={<Button disabled={disabled} aria-label={`移除关联 ${link.target_id}`} onClick={() => void mutate(async () => {
        assertCanWrite();
        await api.raw.DELETE("/links/{linkId}", { params: { path: { linkId: link.id }, query: { expected_version: link.version } } });
      })}>移除</Button>}>{link.target_id}</ListItem>)}
    </List>
    {links.data?.items.length === 0 && <Empty title="暂无资料关联" />}
    <p>目标删除或文件失踪后，关联保留原目标 ID；可移除后重新关联。</p>
    <div className="flex gap-2">
      <Button disabled={disabled || linkPage.previous.length === 0} onClick={() => linkDispatch({ type: "previous" })}>上一页关联</Button>
      <Button disabled={disabled || !links.data?.next_cursor} onClick={() => linkDispatch({ type: "next", nextCursor: links.data?.next_cursor ?? null })}>下一页关联</Button>
    </div>
  </section>;
}
