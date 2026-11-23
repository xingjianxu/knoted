import { useEffect, useId, useReducer, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, ConfirmDialog, Empty, Form, Input, SectionGroup, Spinner, Timeline } from "@bixlab/mtone";
import { ApiRequestError } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../../lib/cursor-state";
import { queryKeys, type QueryClientBundle } from "../../../lib/query";
import { activitiesQuery, revisionsQuery, replaceCachedActivity, type Activity } from "./queries";
const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试";
function validBody(body: string) { return !!body.trim() && Array.from(body).length <= 10_000 && !body.includes("\0"); }
/** Keyed by task ID at its consumer, so late responses cannot replace another task's content or drafts. */
export function ActivityTimeline({ bundle, taskId, disabled }: { bundle: QueryClientBundle; taskId: string; disabled: boolean }) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState(taskId));
  const query = useQuery(activitiesQuery(bundle, taskId, page.cursor));
  const composerId = useId();
  const composer = useRef<HTMLTextAreaElement>(null);
  const disclosure = useRef<HTMLButtonElement>(null);
  const [composing, setComposing] = useState(false);
  const [body, setBody] = useState(""); const [error, setError] = useState("");
  const [busy, setBusy] = useState(false); const writing = useRef(false);
  const locked = disabled || busy;
  useEffect(() => {
    if (composing) composer.current?.focus();
  }, [composing]);
  async function mutate(operation: () => Promise<unknown>) {
    if (disabled || writing.current) throw new Error("当前事项不可写或操作正在进行");
    writing.current = true; setBusy(true);
    try { await operation(); await bundle.queryClient.invalidateQueries({ queryKey: queryKeys.tasks.activities(taskId) }); }
    finally { writing.current = false; setBusy(false); }
  }
  return <SectionGroup dividers={false}>
    <SectionGroup.Section
      title="跟进时间线"
      headingLevel={2}
      actions={
        <Button ref={disclosure} disabled={disabled} aria-disabled={locked} aria-expanded={composing} aria-controls={composerId}
          onClick={() => { if (!locked) setComposing((open) => !open); }}>{composing ? "收起跟进输入" : "添加跟进"}</Button>
      }
    >
      <div className="space-y-3">
        <Form id={composerId} hidden={!composing} onSubmit={(event) => { event.preventDefault(); if (locked || !validBody(body)) return; setError(""); void mutate(async () => {
          const { data } = await bundle.api.raw.POST("/tasks/{taskId}/activities", { params: { path: { taskId } }, body: { body } });
          if (!data) throw new Error("无法读取新增跟进");
          bundle.queryClient.setQueryData(activitiesQuery(bundle, taskId, null).queryKey, (previous) => previous
            ? { ...previous, items: [data, ...previous.items.filter((item) => item.id !== data.id)] } : previous);
        }).then(() => { setBody(""); setComposing(false); disclosure.current?.focus(); dispatch({ type: "filter", filterKey: `${page.filterKey}:created` }); }).catch((cause: unknown) => setError(message(cause))); }} className="gap-2">
          <Form.Item label="新增跟进正文" description="1—10000个字符"><Input.TextArea ref={composer} value={body} disabled={locked} onChange={(event) => setBody(event.target.value)} /></Form.Item>
          <Button type="submit" disabled={locked || !validBody(body)} variant="blue">提交跟进</Button>
          {error && <Alert tone="error">{error}</Alert>}
        </Form>
        {query.isFetching && <Spinner label="正在读取跟进" />}
        {query.error && <Alert tone="error">{query.error.message}<Button disabled={query.isFetching} onClick={() => void query.refetch()}>重试跟进列表</Button></Alert>}
        <Timeline aria-label="事项跟进记录" items={(query.data?.items ?? []).map((activity) => ({ key: activity.id,
          title: <>{activity.author_kind === "system" ? "系统事件" : "人工跟进"} · <time dateTime={activity.created_at}>{new Date(activity.created_at).toLocaleString("zh-CN", { timeZone: "Asia/Shanghai" })}</time></>,
          content: <ActivityEntry bundle={bundle} taskId={taskId} activity={activity} disabled={locked} mutate={mutate} refreshActivity={async () => {
            const result = await query.refetch();
            if (result.error) throw result.error;
            return result.data?.items.find((item) => item.id === activity.id);
          }} />,
        }))} />
        {query.isSuccess && query.data.items.length === 0 && <Empty title="暂无跟进记录" />}
        <div className="flex flex-wrap gap-2">
          <Button disabled={query.isFetching || busy || !page.previous.length} onClick={() => dispatch({ type: "previous" })}>上一页跟进</Button>
          <Button disabled={query.isFetching || busy || !query.data?.next_cursor || query.data.next_cursor === page.cursor} onClick={() => dispatch({ type: "next", nextCursor: query.data?.next_cursor ?? null })}>下一页跟进</Button>
          <Button disabled={query.isFetching || busy} onClick={() => void query.refetch()}>刷新跟进</Button>
        </div>
      </div>
    </SectionGroup.Section>
  </SectionGroup>;
}
function ActivityEntry({ bundle, taskId, activity, disabled, mutate, refreshActivity }: {
  bundle: QueryClientBundle; taskId: string; activity: Activity; disabled: boolean; mutate: (operation: () => Promise<unknown>) => Promise<void>; refreshActivity: () => Promise<Activity | undefined>;
}) {
  // Capture the body AND base version when editing starts. Ordinary query refreshes must not rebase this draft.
  const [draft, setDraft] = useState<{ body: string; version: number }>();
  const [error, setError] = useState(""); const [conflict, setConflict] = useState(false);
  const [refreshing, setRefreshing] = useState(false); const [remoteBody, setRemoteBody] = useState<string>();
  const manual = activity.author_kind === "manual"; const deleted = activity.deleted_at !== null;
  function recordError(cause: unknown) {
    const stale = cause instanceof ApiRequestError && cause.status === 409;
    if (stale && draft) setConflict(true);
    setError(`${message(cause)}${stale && draft ? "；草稿已保留，请重新读取跟进版本并核对后重试。" : ""}`);
  }
  async function refreshBase() {
    setRefreshing(true); setError("");
    try {
      const latest = await refreshActivity();
      if (!latest || latest.deleted_at) throw new Error("跟进已删除或不在当前页，请取消编辑并重新打开记录");
      setRemoteBody(latest.body); setDraft((value) => value ? { ...value, version: latest.version } : value); setConflict(false);
    } catch (cause) { setError(`刷新失败：${message(cause)}`); }
    finally { setRefreshing(false); }
  }
  return <div className="space-y-2">
    <p className="wrap-anywhere whitespace-pre-wrap">{deleted ? "此跟进已删除" : activity.body}</p>
    {manual && !deleted && (draft ? <Form className="gap-2" onSubmit={(event) => { event.preventDefault(); if (disabled || conflict || refreshing || !validBody(draft.body)) return; setError(""); void mutate(async () => {
      const { data } = await bundle.api.raw.PATCH("/tasks/{taskId}/activities/{activityId}", { params: { path: { taskId, activityId: activity.id } }, body: { body: draft.body, expected_version: draft.version } });
      if (!data) throw new Error("无法读取已保存跟进");
      replaceCachedActivity(bundle, taskId, data);
    }).then(() => { setDraft(undefined); setRemoteBody(undefined); }).catch(recordError); }}>
      <Form.Item label="编辑跟进正文"><Input.TextArea value={draft.body} disabled={disabled || refreshing} onChange={(event) => setDraft({ ...draft, body: event.target.value })} /></Form.Item>
      {remoteBody !== undefined && <Alert tone="info">当前服务端正文：{remoteBody}。已载入基础版本 {draft.version}，请核对保留草稿后再保存。</Alert>}
      <Button type="submit" disabled={disabled || conflict || refreshing || !validBody(draft.body)}>保存跟进</Button>
      <Button disabled={disabled || refreshing} onClick={() => { setDraft(undefined); setConflict(false); setRemoteBody(undefined); setError(""); }}>取消编辑跟进</Button>
      {conflict && <Button disabled={disabled || refreshing} onClick={() => void refreshBase()}>重新读取跟进版本</Button>}
    </Form> : <Button disabled={disabled} onClick={() => { setDraft({ body: activity.body, version: activity.version }); setConflict(false); setError(""); }}>编辑跟进</Button>)}
    {manual && !deleted && <ConfirmDialog title="删除跟进？" description="保留删除标记，旧正文仅能在修订中查看。" confirmText="确认删除跟进" cancelText="取消" danger
      trigger={<Button danger disabled={disabled || !!draft}>删除跟进</Button>} onConfirm={async () => {
        setError(""); try { await mutate(async () => {
          await bundle.api.raw.DELETE("/tasks/{taskId}/activities/{activityId}", { params: { path: { taskId, activityId: activity.id }, query: { expected_version: activity.version } } });
          // 204 confirms deletion; immediately hide the body even if the canonical refresh fails.
          replaceCachedActivity(bundle, taskId, { ...activity, body: "", deleted_at: new Date().toISOString() });
        }); }
        catch (cause) { recordError(cause); throw cause; }
      }} />}
    {error && <Alert tone="error">{error}</Alert>}
    {manual && <ActivityRevisions key={`${activity.id}:${activity.version}:${activity.deleted_at ?? "active"}`} bundle={bundle} taskId={taskId} activity={activity} />}
  </div>;
}
function ActivityRevisions({ bundle, taskId, activity }: { bundle: QueryClientBundle; taskId: string; activity: Activity }) {
  const [open, setOpen] = useState(false);
  const revisionId = useId();
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState(activity.id));
  const query = useQuery({ ...revisionsQuery(bundle, taskId, activity, page.cursor), enabled: open });
  return <div className="space-y-2">
    <Button aria-expanded={open} aria-controls={revisionId} onClick={() => setOpen(!open)}>{open ? "收起修订" : "查看修订"}</Button>
    <div id={revisionId} hidden={!open} className="space-y-2">
      {open && <>
        {query.isFetching && <Spinner label="正在读取修订" />}
        {query.error && <Alert tone="error">{query.error.message}<Button disabled={query.isFetching} onClick={() => void query.refetch()}>重试修订</Button></Alert>}
        <Timeline aria-label="跟进修订历史" items={(query.data?.items ?? []).map((revision) => ({ key: revision.id, title: `版本 ${revision.version}`, content: <p className="wrap-anywhere whitespace-pre-wrap">{revision.body}</p> }))} />
        {query.isSuccess && query.data.items.length === 0 && <Empty title="暂无修订" />}
        <Button disabled={query.isFetching || !page.previous.length} onClick={() => dispatch({ type: "previous" })}>上一页修订</Button>
        <Button disabled={query.isFetching || !query.data?.next_cursor || query.data.next_cursor === page.cursor} onClick={() => dispatch({ type: "next", nextCursor: query.data?.next_cursor ?? null })}>下一页修订</Button>
      </>}
    </div>
  </div>;
}
