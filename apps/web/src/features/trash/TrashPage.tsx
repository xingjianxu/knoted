import { Alert, Button, Dialog, Empty, List, ListItem, PageLayout, Paragraph, Select, Spinner, Tag, Toolbar } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useReducer, useRef, useState } from "react";
import { ApiRequestError, type components } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";

type Item = components["schemas"]["TrashItem"];
type Page = components["schemas"]["TrashPage"];
type EntityType = components["schemas"]["TrashEntityType"];
const labels = { note: "笔记", project: "项目", task: "事项" };

export function TrashPage({ bundle }: { bundle: QueryClientBundle }) {
  const [filter, setFilter] = useState<EntityType | "all">("all");
  const [paging, dispatch] = useReducer(cursorReducer, createCursorState("all"));
  const [restoredItem, setRestoredItem] = useState<Item>();
  const pageLayout = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!restoredItem) return;
    const frame = window.requestAnimationFrame(() => {
      const heading = pageLayout.current?.querySelector<HTMLElement>("h1");
      if (!heading) return;
      heading.tabIndex = -1;
      heading.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [restoredItem]);
  const key = [...queryKeys.trash(), { entity_type: filter, cursor: paging.cursor }];
  const page = useQuery({ queryKey: key, retry: false, refetchOnMount: "always", queryFn: async ({ signal }) => {
    const { data } = await bundle.api.raw.GET("/trash", { signal, params: { query: { entity_type: filter === "all" ? undefined : filter, cursor: paging.cursor ?? undefined, limit: 50 } } });
    if (!data) throw new Error("无法读取回收站");
    return data;
  } });
  function restored(item: Item) {
    // The write already succeeded. Remove its row from
    // every trash cache before starting a fresh read. Other rows keep identity.
    bundle.queryClient.setQueriesData<Page>({ queryKey: queryKeys.trash() }, old => old ? { ...old, items: old.items.filter(row => row.entity_type !== item.entity_type || row.id !== item.id) } : old);
    setRestoredItem(item);
    void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.trash() });
    for (const queryKey of [queryKeys.notes.all(), queryKeys.projects.all(), queryKeys.tasks.all(), queryKeys.dashboard()]) {
      void bundle.queryClient.invalidateQueries({ queryKey });
    }
  }
  return <section aria-labelledby="trash-title">
    <PageLayout ref={pageLayout} title={<span id="trash-title">回收站</span>}>
      <div className="grid gap-4">
        <Toolbar
          background="white"
          aria-label="回收站筛选"
          start={
            <>
              <Select label="对象类型" value={filter}
                options={[{ value: "all", label: "全部" }, { value: "note", label: "笔记" }, { value: "project", label: "项目" }, { value: "task", label: "事项" }]}
                onValueChange={value => {
                  if (value) {
                    setFilter(value as EntityType | "all");
                    dispatch({ type: "filter", filterKey: value });
                  }
                }} />
              <Button disabled={page.isFetching} onClick={() => { void page.refetch(); }}>刷新列表</Button>
            </>
          }
        />
        <Paragraph muted className="text-sm">删除满 30 天后具备清理资格。恢复时会重新校验版本与关系。</Paragraph>
        {restoredItem && <p role="status" key={`${restoredItem.entity_type}:${restoredItem.id}`}>已恢复 {restoredItem.title}</p>}
        {page.isFetching && <Spinner label="加载回收站" />}
        {page.isError && <Alert tone="error" title={page.data ? "列表刷新失败" : "回收站暂时不可用"}>请重试读取列表；已经成功的恢复操作无需重复提交。</Alert>}
        {!!page.data?.items.length && <List aria-label="回收对象">
          {page.data.items.map(item => <RestoreAction key={`${item.entity_type}:${item.id}`} bundle={bundle} item={item} onRestored={restored} />)}
        </List>}
        {page.isSuccess && !page.isFetching && page.data.items.length === 0 && <Empty title="回收站为空" description="当前筛选下没有软删除对象。" />}
        {(page.data || paging.previous.length > 0) && <nav aria-label="回收站翻页" className="flex flex-wrap gap-2">
          <Button disabled={page.isFetching || paging.previous.length === 0} onClick={() => dispatch({ type: "previous" })}>上一页</Button>
          <Button disabled={page.isFetching || page.isError || !page.data?.next_cursor || page.data.next_cursor === paging.cursor}
            onClick={() => dispatch({ type: "next", nextCursor: page.data?.next_cursor ?? null })}>下一页</Button>
        </nav>}
      </div>
    </PageLayout>
  </section>;
}

function RestoreAction({ bundle, item, onRestored }: { bundle: QueryClientBundle; item: Item; onRestored: (item: Item) => void }) {
  const isNote = item.entity_type === "note";
  const [position, setPosition] = useState("original");
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const writing = useRef(false);
  const root = isNote && (!item.can_restore || position === "root");
  async function restore() {
    if (writing.current) return;
    writing.current = true;
    setBusy(true);
    setError("");
    try {
      // Prevent a pre-write snapshot arriving after the successful cache edit.
      await bundle.queryClient.cancelQueries({ queryKey: queryKeys.trash() });
      const body = { expected_version: item.version };
      if (item.entity_type === "note") await bundle.api.raw.POST("/notes/{noteId}/restore", { params: { path: { noteId: item.id } }, body: root ? { ...body, restore_parent: { parent_id: null } } : body });
      else if (item.entity_type === "project") await bundle.api.raw.POST("/projects/{projectId}/restore", { params: { path: { projectId: item.id } }, body });
      else await bundle.api.raw.POST("/tasks/{taskId}/restore", { params: { path: { taskId: item.id } }, body });
    } catch (cause) {
      setError(cause instanceof ApiRequestError && cause.status === 409 ? "版本或恢复条件已变化，请刷新列表后重试。" : cause instanceof ApiRequestError && cause.status === 404 ? "对象已清理或不存在，请刷新列表。" : "恢复失败，请稍后重试。");
      return;
    } finally {
      writing.current = false;
      setBusy(false);
    }
    setConfirm(false);
    onRestored(item);
  }
  return <ListItem><section aria-label={`回收对象 ${item.title}`} className="flex flex-wrap items-start justify-between gap-3">
    <div className="min-w-0 space-y-1">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-medium wrap-anywhere">{item.title}</h2>
        <Tag>{labels[item.entity_type]}</Tag>
      </div>
      <p className="text-sm text-muted">删除时间：<time dateTime={item.deleted_at}>{new Date(item.deleted_at).toLocaleString("zh-CN", { timeZone: "Asia/Shanghai" })}</time>（上海）</p>
      {item.reason && <p className="wrap-anywhere">{item.reason}</p>}
    </div>
    <Dialog title={`恢复 ${item.title}`} open={confirm} onOpenChange={value => { if (!writing.current) setConfirm(value); }}
      trigger={<Button aria-label={`恢复 ${item.title}`} disabled={busy || (!item.can_restore && !isNote)}>恢复…</Button>}
      footer={<>
        <Button disabled={busy} onClick={() => setConfirm(false)}>取消</Button>
        <Button variant="solid" loading={busy} onClick={() => { void restore(); }}>确认恢复</Button>
      </>}>
      <div className="grid gap-3">
        {isNote && <Select label={`恢复位置 ${item.title}`} value={root ? "root" : "original"} disabled={busy}
          options={item.can_restore ? [{ value: "original", label: "原位置" }, { value: "root", label: "根目录" }] : [{ value: "root", label: "根目录" }]}
          onValueChange={value => { if (value) setPosition(value); }} />}
        <p>确认将{labels[item.entity_type]}“{item.title}”恢复{root ? "到根目录" : "到原位置"}？</p>
        {error && <Alert tone="error">{error}</Alert>}
      </div>
    </Dialog>
  </section></ListItem>;
}
