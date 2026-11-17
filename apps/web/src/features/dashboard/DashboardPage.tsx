import { Alert, Badge, Button, Card, Empty, List, ListItem, Spinner } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { dateBoundary } from "../tasks/dates";

export function DashboardPage({ bundle, onSelectNote, onSelectTask }: {
  bundle: QueryClientBundle;
  onSelectNote: (id: string) => void;
  onSelectTask: (id: string) => void;
}) {
  const summary = useQuery({ queryKey: queryKeys.dashboard(), retry: false, refetchOnMount: "always", queryFn: async ({ signal }) => {
    const { data } = await bundle.api.raw.GET("/dashboard", { signal });
    if (!data) throw new Error("无法读取工作概览");
    return data;
  } });
  // Never display a previously cached overdue list while refreshing on entry.
  if (summary.isFetching) return <Spinner label="加载工作概览" />;
  if (summary.isError || !summary.data) return <Alert tone="error" title="工作概览暂时不可用">请稍后重试。<Button onClick={() => { void summary.refetch(); }}>重试</Button></Alert>;
  const data = summary.data;
  return <main aria-label="工作概览" className="grid gap-4">
    <h1>工作概览</h1>
    <Card title="当前概况">
      <p>笔记：{data.counts.notes} · 未完成事项：{data.counts.tasks_open} · 活跃项目：{data.counts.projects_active}</p>
      <p>今日到期：{data.counts.tasks_due_today} · 失败任务：{data.job_failures}</p>
      <p>待办：{data.tasks_by_status.todo} · 进行中：{data.tasks_by_status.in_progress} · 已完成：{data.tasks_by_status.done} · 已取消：{data.tasks_by_status.cancelled}</p>
      <p>文件：{data.counts.files}（文件浏览与解析暂未开放）</p>
      {!data.indexing_enabled && <Badge>索引未启用</Badge>}
    </Card>
    <Card title="到期与逾期事项">
      <List aria-label="到期事项">{data.due_tasks.map(task => {
        const boundary = task.due ? dateBoundary(task.due, true) : null;
        const overdue = task.status !== "done" && task.status !== "cancelled" && boundary !== null && summary.dataUpdatedAt >= boundary;
        return <ListItem key={task.id} description={task.due ? `${task.due.date}${task.due.time ? ` ${task.due.time}` : ""}（上海）` : undefined}
          actions={<Button aria-label={`打开事项 ${task.title}`} onClick={() => onSelectTask(task.id)}>打开</Button>}>
          {task.title} <Badge>{overdue ? "已逾期" : "今日到期"}</Badge>
        </ListItem>;
      })}</List>
      {data.due_tasks.length === 0 && <Empty title="暂无到期事项" description="没有需要提示的到期或逾期事项。" />}
    </Card>
    <Card title="最近笔记">
      <List aria-label="最近笔记">{data.recent_notes.map(note => <ListItem key={note.id} actions={<Button aria-label={`打开笔记 ${note.title}`} onClick={() => onSelectNote(note.id)}>打开</Button>}>{note.title}</ListItem>)}</List>
      {data.recent_notes.length === 0 && <Empty title="暂无最近笔记" description="创建笔记后会在这里显示。" />}
    </Card>
    <Card title="最近事项">
      <List aria-label="最近事项">{data.recent_tasks.map(task => <ListItem key={task.id} actions={<Button aria-label={`打开事项 ${task.title}`} onClick={() => onSelectTask(task.id)}>打开</Button>}>{task.title}</ListItem>)}</List>
      {data.recent_tasks.length === 0 && <Empty title="暂无最近事项" />}
    </Card>
  </main>;
}
