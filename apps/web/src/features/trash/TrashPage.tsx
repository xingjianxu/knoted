import { Alert, Button, Dialog, Empty, List, ListItem, Select, Spinner } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { useReducer, useState } from "react";
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
  const [notice, setNotice] = useState("");
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
    setNotice(`已恢复 ${item.title}`);
    void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.trash() });
    for (const queryKey of [queryKeys.notes.all(), queryKeys.projects.all(), queryKeys.tasks.all(), queryKeys.dashboard()]) {
      void bundle.queryClient.invalidateQueries({ queryKey });
    }
  }
  return <main aria-label="回收站" className="grid gap-4">
    <h1>回收站</h1>
    <p>软删除对象保留原有关联，删除满 30 天后具备清理资格。仍在列表中的对象可以尝试恢复，恢复时将再次校验版本与关系。</p>
    <Select label="对象类型" value={filter} options={[{ value: "all", label: "全部" }, { value: "note", label: "笔记" }, { value: "project", label: "项目" }, { value: "task", label: "事项" }]} onValueChange={value => { if (value) { setFilter(value as EntityType | "all"); dispatch({ type: "filter", filterKey: value }); } }} />
    <Button disabled={page.isFetching} onClick={() => { void page.refetch(); }}>刷新列表</Button>
    {notice && <p role="status">{notice}</p>}
    {page.isFetching && <Spinner label="加载回收站" />}
    {page.isError && <Alert tone="error" title={page.data ? "列表刷新失败" : "回收站暂时不可用"}>请重试读取列表；已经成功的恢复操作无需重复提交。</Alert>}
    <List aria-label="回收对象">{page.data?.items.map(item => <RestoreAction key={`${item.entity_type}:${item.id}`} bundle={bundle} item={item} onRestored={restored} />)}</List>
    {page.data?.items.length === 0 && <Empty title="回收站为空" description="当前筛选下没有软删除对象；归档对象请到原列表查看。" />}
    <div className="flex gap-2">
      <Button disabled={page.isFetching || paging.previous.length === 0} onClick={() => dispatch({ type: "previous" })}>上一页</Button>
      <Button disabled={page.isFetching || !page.data?.next_cursor} onClick={() => dispatch({ type: "next", nextCursor: page.data?.next_cursor ?? null })}>下一页</Button>
    </div>
  </main>;
}

function RestoreAction({ bundle, item, onRestored }: { bundle: QueryClientBundle; item: Item; onRestored: (item: Item) => void }) {
  const [position, setPosition] = useState("original");
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const root = item.entity_type === "note" && position === "root";
  async function restore() {
    setBusy(true); setError("");
    try {
      // Prevent a pre-write snapshot arriving after the successful cache edit.
      await bundle.queryClient.cancelQueries({ queryKey: queryKeys.trash() });
      const body = { expected_version: item.version };
      if (item.entity_type === "note") await bundle.api.raw.POST("/notes/{noteId}/restore", { params: { path: { noteId: item.id } }, body: root ? { ...body, restore_parent: { parent_id: null } } : body });
      else if (item.entity_type === "project") await bundle.api.raw.POST("/projects/{projectId}/restore", { params: { path: { projectId: item.id } }, body });
      else await bundle.api.raw.POST("/tasks/{taskId}/restore", { params: { path: { taskId: item.id } }, body });
    } catch (cause) {
      setError(cause instanceof ApiRequestError && cause.status === 409 ? "版本或恢复条件已变化，请刷新列表后重试。" : cause instanceof ApiRequestError && cause.status === 404 ? "对象已清理或不存在，请刷新列表。" : "恢复失败，请稍后重试。");
      setBusy(false); setConfirm(false); return;
    }
    setBusy(false); setConfirm(false);
    onRestored(item);
  }
  return <ListItem><section aria-label={`回收对象 ${item.title}`} className="grid gap-2">
    <h2>{item.title}</h2><p>{labels[item.entity_type]} · 删除时间：{new Date(item.deleted_at).toLocaleString("zh-CN", { timeZone: "Asia/Shanghai" })}（上海）</p>
    {item.reason && <p>{item.reason}</p>}
    {item.entity_type === "note" && <Select label={`恢复位置 ${item.title}`} value={position} disabled={busy} options={[{ value: "original", label: "原位置" }, { value: "root", label: "根目录" }]} onValueChange={value => { if (value) setPosition(value); }} />}
    {error && <Alert tone="error">{error}</Alert>}
    <Button aria-label={`恢复 ${item.title}`} disabled={busy || (!item.can_restore && !root)} onClick={() => setConfirm(true)}>恢复</Button>
    <Dialog title={`恢复 ${item.title}`} open={confirm} onOpenChange={value => { if (!busy) setConfirm(value); }} footer={<><Button disabled={busy} onClick={() => setConfirm(false)}>取消</Button><Button disabled={busy} onClick={() => { void restore(); }}>确认恢复</Button></>}>
      <p>确认将{labels[item.entity_type]}“{item.title}”恢复{root ? "到根目录" : "到原位置"}？</p>
    </Dialog>
  </section></ListItem>;
}
