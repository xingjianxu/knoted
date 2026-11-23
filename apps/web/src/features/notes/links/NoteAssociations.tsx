import { useReducer, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, Button, Drawer, Empty, Form, Input, List, ListItem, SectionGroup, Select, Spinner } from "@bixlab/mtone";
import type { ApiClient, components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../../lib/cursor-state";

type EntityType = components["schemas"]["EntityEndType"];
type Section = "tags" | "links";
const entityLabels = { note: "笔记", file: "文件", project: "项目", task: "事项" } satisfies Record<EntityType, string>;
export interface NoteAssociationsProps {
  noteId: string;
  api: ApiClient;
  readOnly?: boolean;
  /** Fresh workspace policy; readOnly alone cannot stop an awaited continuation. */
  assertCanWrite?: () => void;
}
/** Keep association drafts scoped to the current note while the drawer is closed. */
export function NoteAssociations(props: NoteAssociationsProps) {
  return <NoteAssociationsContent key={props.noteId} {...props} />;
}
function NoteAssociationsContent({ noteId, api, readOnly = false, assertCanWrite = () => {} }: NoteAssociationsProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [targetType, setTargetType] = useState<EntityType>("note");
  const [targetId, setTargetId] = useState("");
  const [editingTags, setEditingTags] = useState(false);
  const [editingLinks, setEditingLinks] = useState(false);
  const [createdTag, setCreatedTag] = useState<{ id: string; name: string }>();
  const pending = useRef(false);
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState({ tags: "", links: "" });
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
    queryKey: ["tag-options", optionsPage.cursor], enabled: editingTags, retry: false,
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
  const tagsDisabled = readOnly || busy || tags.isFetching || tags.isError;
  const optionsDisabled = tagsDisabled || options.isFetching || options.isError;
  const linksDisabled = readOnly || busy || links.isFetching || links.isError;

  async function mutate(section: Section, operation: () => Promise<void>) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setErrors(previous => ({ ...previous, [section]: "" }));
    try {
      await operation();
      await queryClient.invalidateQueries({ queryKey: [...root, section] });
    } catch (cause) {
      setErrors(previous => ({ ...previous, [section]: cause instanceof Error ? cause.message : "操作失败，请重试" }));
    } finally {
      setBusy(false);
      pending.current = false;
    }
  }
  async function assign(tagId: string) {
    assertCanWrite();
    await api.raw.PUT("/entities/{entityType}/{entityId}/tags/{tagId}", { params: { path: { entityType: "note", entityId: noteId, tagId } } });
  }
  const tagChoices = options.data?.items ?? [];
  const tagOptions = tagChoices.map(tag => ({ value: tag.id, label: tag.name }));
  if (createdTag && !tagChoices.some(tag => tag.id === createdTag.id)) {
    tagOptions.unshift({ value: createdTag.id, label: createdTag.name });
  }
  const nextTag = tags.data?.next_cursor ?? null;
  const nextOption = options.data?.next_cursor ?? null;
  const nextLink = links.data?.next_cursor ?? null;

  return <Drawer title="标签与关联" trigger={<Button size="sm" variant="outline">标签与关联</Button>}>
    <SectionGroup aria-label="笔记标签与资料关联" role="region">
      <SectionGroup.Section title="标签" headingLevel={3} aria-labelledby={undefined} aria-label="笔记标签">
        <div className="space-y-3">
        {tags.isFetching && <Spinner label="正在读取标签" />}
        {tags.error && <Alert tone="error">{tags.error.message} <Button size="sm" onClick={() => void tags.refetch()}>重试标签</Button></Alert>}
        {errors.tags && <Alert tone="error">{errors.tags}</Alert>}
        {tags.data?.items.length === 0 && <Empty title="暂无标签" />}
        {!!tags.data?.items.length && <List aria-label="笔记标签列表">
          {tags.data.items.map(tag => <ListItem key={tag.id}
            actions={<Button size="sm" disabled={tagsDisabled} aria-label={`移除标签 ${tag.name}`} onClick={() => void mutate("tags", async () => {
              assertCanWrite();
              await api.raw.DELETE("/entities/{entityType}/{entityId}/tags/{tagId}", { params: { path: { entityType: "note", entityId: noteId, tagId: tag.id } } });
            })}>移除</Button>}><span className="[overflow-wrap:anywhere]">{tag.name}</span></ListItem>)}
        </List>}
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={tags.isFetching || tagPage.previous.length === 0} onClick={() => tagDispatch({ type: "previous" })}>上一页标签</Button>
          <Button size="sm" disabled={tags.isFetching || !nextTag || nextTag === tagPage.cursor} onClick={() => tagDispatch({ type: "next", nextCursor: nextTag })}>下一页标签</Button>
          <Button size="sm" variant="outline" aria-expanded={editingTags} aria-controls="edit-note-tags" onClick={() => setEditingTags(value => !value)}>添加标签</Button>
        </div>
        <div id="edit-note-tags" hidden={!editingTags} className="space-y-3">
          <Form onSubmit={event => {
            event.preventDefault();
            if (tagsDisabled) return;
            void mutate("tags", async () => {
              if (!name.trim()) throw new Error("请输入标签名称");
              assertCanWrite();
              const { data } = await api.raw.POST("/tags", { body: { name: name.trim() } });
              if (!data) throw new Error("无法创建标签");
              // The created ID survives assignment failure and permits an explicit retry.
              setCreatedTag(data);
              setSelectedTag(data.id);
              setName("");
              await assign(data.id);
              await queryClient.invalidateQueries({ queryKey: ["tag-options"] });
            });
          }} className="flex flex-wrap items-end gap-2">
            <Form.Item label="新标签名称"><Input value={name} maxLength={64} onChange={event => setName(event.target.value)} disabled={tagsDisabled} /></Form.Item>
            <Button type="submit" disabled={tagsDisabled}>创建并添加标签</Button>
          </Form>
          {options.isFetching && <Spinner label="正在读取已有标签" />}
          {options.error && <Alert tone="error">{options.error.message} <Button size="sm" onClick={() => void options.refetch()}>重试已有标签</Button></Alert>}
          <div className="flex flex-wrap items-end gap-2">
            <Form.Item label="已有标签"><Select value={selectedTag} onValueChange={setSelectedTag} options={tagOptions} disabled={optionsDisabled} /></Form.Item>
            <Button disabled={tagsDisabled || (optionsDisabled && selectedTag !== createdTag?.id) || !selectedTag}
              onClick={() => { if (selectedTag) void mutate("tags", () => assign(selectedTag)); }}>添加已有标签</Button>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={options.isFetching || optionsPage.previous.length === 0} onClick={() => optionsDispatch({ type: "previous" })}>上一页已有标签</Button>
            <Button size="sm" disabled={options.isFetching || !nextOption || nextOption === optionsPage.cursor} onClick={() => optionsDispatch({ type: "next", nextCursor: nextOption })}>下一页已有标签</Button>
          </div>
        </div>
        </div>
      </SectionGroup.Section>
      <SectionGroup.Section title="资料关联" headingLevel={3}>
        <div className="space-y-3">
        {links.isFetching && <Spinner label="正在读取资料关联" />}
        {links.error && <Alert tone="error">{links.error.message} <Button size="sm" onClick={() => void links.refetch()}>重试资料关联</Button></Alert>}
        {errors.links && <Alert tone="error">{errors.links}</Alert>}
        {links.data?.items.length === 0 && <Empty title="暂无资料关联" />}
        {!!links.data?.items.length && <List aria-label="资料关联列表">
          {links.data.items.map(link => <ListItem key={link.id} description={entityLabels[link.target_type] ?? link.target_type}
            actions={<Button size="sm" disabled={linksDisabled} aria-label={`移除关联 ${link.target_id}`} onClick={() => void mutate("links", async () => {
              assertCanWrite();
              await api.raw.DELETE("/links/{linkId}", { params: { path: { linkId: link.id }, query: { expected_version: link.version } } });
            })}>移除</Button>}><span className="[overflow-wrap:anywhere]">{link.target_id}</span></ListItem>)}
        </List>}
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={links.isFetching || linkPage.previous.length === 0} onClick={() => linkDispatch({ type: "previous" })}>上一页关联</Button>
          <Button size="sm" disabled={links.isFetching || !nextLink || nextLink === linkPage.cursor} onClick={() => linkDispatch({ type: "next", nextCursor: nextLink })}>下一页关联</Button>
          <Button size="sm" variant="outline" aria-expanded={editingLinks} aria-controls="edit-note-links" onClick={() => setEditingLinks(value => !value)}>新建关联</Button>
        </div>
        <div id="edit-note-links" hidden={!editingLinks}>
          <Form onSubmit={event => {
            event.preventDefault();
            if (linksDisabled) return;
            void mutate("links", async () => {
              if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(targetId.trim())) throw new Error("请输入有效的目标 ID");
              assertCanWrite();
              await api.raw.POST("/links", { body: { source_type: "note", source_id: noteId, target_type: targetType, target_id: targetId.trim() } });
              setTargetId("");
            });
          }} className="flex flex-wrap items-end gap-2">
            <Form.Item label="关联目标类型"><Select value={targetType} onValueChange={value => { if (value) setTargetType(value as EntityType); }}
              options={Object.entries(entityLabels).map(([value, label]) => ({ value, label }))} disabled={linksDisabled} /></Form.Item>
            <Form.Item label="关联目标 ID"><Input value={targetId} onChange={event => setTargetId(event.target.value)} disabled={linksDisabled} /></Form.Item>
            <Button type="submit" disabled={linksDisabled}>添加关联</Button>
          </Form>
        </div>
        </div>
      </SectionGroup.Section>
    </SectionGroup>
  </Drawer>;
}
