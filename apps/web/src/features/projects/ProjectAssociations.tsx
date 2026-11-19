import { useId, useReducer, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, Button, Empty, FormField, Input, List, ListItem, Select, Spinner } from "@bixlab/mtone";
import type { ApiClient, components } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";

type EntityType = components["schemas"]["EntityEndType"];
const targetLabels: Record<EntityType, string> = {
  note: "笔记", file: "文件", project: "项目", task: "事项",
};
const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试";

export interface ProjectAssociationsProps {
  projectId: string;
  api: ApiClient;
  /** Archived projects are read-only until unarchived. */
  readOnly?: boolean;
}

export function ProjectAssociations({ projectId, api, readOnly = false }: ProjectAssociationsProps) {
  const queryClient = useQueryClient();
  const tagsId = useId();
  const linksId = useId();
  const [addingTag, setAddingTag] = useState(false);
  const [addingLink, setAddingLink] = useState(false);
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [targetType, setTargetType] = useState<EntityType>("note");
  const [targetId, setTargetId] = useState("");
  const [busy, setBusy] = useState<"tags" | "links" | null>(null);
  const [tagError, setTagError] = useState("");
  const [linkError, setLinkError] = useState("");
  const [tagPage, tagDispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const [linkPage, linkDispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const [optionsPage, optionsDispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const root = ["project-associations", projectId];

  const tags = useQuery({
    queryKey: [...root, "tags", tagPage.cursor],
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await api.raw.GET("/entities/{entityType}/{entityId}/tags", {
        signal,
        params: {
          path: { entityType: "project", entityId: projectId },
          query: { limit: 50, cursor: tagPage.cursor ?? undefined },
        },
      });
      if (!data) throw new Error("无法读取项目标签");
      return data;
    },
  });
  const options = useQuery({
    queryKey: ["tag-options", optionsPage.cursor],
    enabled: addingTag && !readOnly,
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await api.raw.GET("/tags", {
        signal,
        params: { query: { limit: 50, cursor: optionsPage.cursor ?? undefined } },
      });
      if (!data) throw new Error("无法读取已有标签");
      return data;
    },
  });
  const links = useQuery({
    queryKey: [...root, "links", linkPage.cursor],
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await api.raw.GET("/links", {
        signal,
        params: {
          query: {
            source_type: "project",
            source_id: projectId,
            limit: 50,
            cursor: linkPage.cursor ?? undefined,
          },
        },
      });
      if (!data) throw new Error("无法读取资料关联");
      return data;
    },
  });

  const tagsLocked = readOnly || busy !== null || tags.isFetching || tags.isError;
  const linksLocked = readOnly || busy !== null || links.isFetching || links.isError;
  const optionsLocked = tagsLocked || options.isFetching || options.isError;
  async function mutate(group: "tags" | "links", operation: () => Promise<void>) {
    if (group === "tags" ? tagsLocked : linksLocked) return;
    const setError = group === "tags" ? setTagError : setLinkError;
    setBusy(group);
    setError("");
    try {
      await operation();
      await queryClient.invalidateQueries({ queryKey: [...root, group] });
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section aria-label="项目标签与资料关联" className="grid items-start gap-6 md:grid-cols-2">
      <section aria-label="项目标签" className="min-w-0 space-y-3">
        <header className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="section-title">标签</h2>
          {!readOnly && <Button disabled={busy === "tags"} aria-expanded={addingTag} aria-controls={tagsId}
            onClick={() => setAddingTag((open) => !open)}>{addingTag ? "收起标签选项" : "添加标签"}</Button>}
        </header>
        {tags.isFetching && <Spinner label="正在读取项目标签" />}
        {tags.error && <Alert tone="error">{message(tags.error)}
          <Button disabled={tags.isFetching} onClick={() => void tags.refetch()}>重试读取项目标签</Button>
        </Alert>}
        {!!tags.data?.items.length && <List aria-label="项目标签">
          {tags.data.items.map((tag) => (
            <ListItem key={tag.id} actions={<Button disabled={tagsLocked} aria-label={`移除标签 ${tag.name}`}
              onClick={() => void mutate("tags", async () => {
                await api.raw.DELETE("/entities/{entityType}/{entityId}/tags/{tagId}", {
                  params: { path: { entityType: "project", entityId: projectId, tagId: tag.id } },
                });
              })}>移除</Button>}>
              <span className="wrap-anywhere">{tag.name}</span>
            </ListItem>
          ))}
        </List>}
        {tags.isSuccess && tags.data.items.length === 0 && <Empty title="暂无标签" />}
        {(tags.data || tagPage.previous.length > 0) && <nav aria-label="项目标签分页" className="flex flex-wrap gap-2">
          <Button disabled={tags.isFetching || !tagPage.previous.length}
            onClick={() => tagDispatch({ type: "previous" })}>上一页标签</Button>
          <Button disabled={tags.isFetching || !tags.data?.next_cursor || tags.data.next_cursor === tagPage.cursor}
            onClick={() => tagDispatch({ type: "next", nextCursor: tags.data?.next_cursor ?? null })}>下一页标签</Button>
        </nav>}
        <div id={tagsId} hidden={readOnly || !addingTag} className="space-y-2">
          {options.isFetching && addingTag && <Spinner label="正在读取已有标签" />}
          {options.error && <Alert tone="error">{message(options.error)}
            <Button disabled={options.isFetching} onClick={() => void options.refetch()}>重试读取标签选项</Button>
          </Alert>}
          <div className="flex flex-wrap items-end gap-2">
            <FormField label="已有标签"><Select value={selectedTag} onValueChange={setSelectedTag}
              options={(options.data?.items ?? []).map((tag) => ({ value: tag.id, label: tag.name }))}
              disabled={optionsLocked} /></FormField>
            <Button disabled={optionsLocked || !selectedTag} onClick={() => {
              if (!selectedTag) return;
              void mutate("tags", async () => {
                await api.raw.PUT("/entities/{entityType}/{entityId}/tags/{tagId}", {
                  params: { path: { entityType: "project", entityId: projectId, tagId: selectedTag } },
                });
                setSelectedTag(null);
              });
            }}>添加已有标签</Button>
          </div>
          {(options.data || optionsPage.previous.length > 0) && <nav aria-label="已有标签选项分页" className="flex flex-wrap gap-2">
            <Button disabled={options.isFetching || !optionsPage.previous.length}
              onClick={() => {
                setSelectedTag(null);
                optionsDispatch({ type: "previous" });
              }}>上一页已有标签</Button>
            <Button disabled={options.isFetching || !options.data?.next_cursor || options.data.next_cursor === optionsPage.cursor}
              onClick={() => {
                setSelectedTag(null);
                optionsDispatch({ type: "next", nextCursor: options.data?.next_cursor ?? null });
              }}>下一页已有标签</Button>
          </nav>}
        </div>
        {tagError && <Alert tone="error">{tagError}</Alert>}
      </section>
      <section aria-label="资料关联" className="min-w-0 space-y-3">
        <header className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="section-title">资料关联</h2>
          {!readOnly && <Button disabled={busy === "links"} aria-expanded={addingLink} aria-controls={linksId}
            onClick={() => setAddingLink((open) => !open)}>{addingLink ? "收起关联输入" : "添加资料"}</Button>}
        </header>
        {links.isFetching && <Spinner label="正在读取资料关联" />}
        {links.error && <Alert tone="error">{message(links.error)}
          <Button disabled={links.isFetching} onClick={() => void links.refetch()}>重试读取资料关联</Button>
        </Alert>}
        {!!links.data?.items.length && <List aria-label="资料关联">
          {links.data.items.map((link) => (
            <ListItem key={link.id} description={targetLabels[link.target_type]}
              actions={<Button disabled={linksLocked} aria-label={`移除关联 ${link.target_id}`}
                onClick={() => void mutate("links", async () => {
                  await api.raw.DELETE("/links/{linkId}", {
                    params: { path: { linkId: link.id }, query: { expected_version: link.version } },
                  });
                })}>移除</Button>}>
              <span className="wrap-anywhere">{link.target_id}</span>
            </ListItem>
          ))}
        </List>}
        {links.isSuccess && links.data.items.length === 0 && <Empty title="暂无资料关联" />}
        {(links.data || linkPage.previous.length > 0) && <nav aria-label="资料关联分页" className="flex flex-wrap gap-2">
          <Button disabled={links.isFetching || !linkPage.previous.length}
            onClick={() => linkDispatch({ type: "previous" })}>上一页关联</Button>
          <Button disabled={links.isFetching || !links.data?.next_cursor || links.data.next_cursor === linkPage.cursor}
            onClick={() => linkDispatch({ type: "next", nextCursor: links.data?.next_cursor ?? null })}>下一页关联</Button>
        </nav>}
        <div id={linksId} hidden={readOnly || !addingLink}>
          <form className="flex flex-wrap items-end gap-2" onSubmit={(event) => {
            event.preventDefault();
            void mutate("links", async () => {
              if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(targetId.trim())) {
                throw new Error("请输入有效的目标 ID");
              }
              await api.raw.POST("/links", { body: {
                source_type: "project", source_id: projectId, target_type: targetType, target_id: targetId.trim(),
              } });
              setTargetId("");
            });
          }}>
            <FormField label="关联目标类型"><Select value={targetType}
              onValueChange={(value) => { if (value) setTargetType(value as EntityType); }}
              options={Object.entries(targetLabels).map(([value, label]) => ({ value, label }))}
              disabled={linksLocked} /></FormField>
            <FormField label="关联目标 ID">
              <Input value={targetId} onChange={(event) => setTargetId(event.target.value)} disabled={linksLocked} />
            </FormField>
            <Button type="submit" disabled={linksLocked}>添加关联</Button>
          </form>
        </div>
        {linkError && <Alert tone="error">{linkError}</Alert>}
      </section>
    </section>
  );
}
