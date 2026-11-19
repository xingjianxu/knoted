import { Alert, Badge, Button, Card, Empty, List, ListItem, Spinner } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { dateBoundary } from "../tasks/dates";

function Count({ label, value }: { label: string; value: number }) {
  return <div role="group" aria-label={label} className="min-w-0">
    <dt className="text-sm text-muted">{label}</dt>
    <dd className="wrap-anywhere text-2xl font-semibold tabular-nums">{value}</dd>
  </div>;
}

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
  // Cached overdue labels may be stale when the summary is opened again.
  if (summary.isFetching) return <Spinner label="加载工作概览" />;
  if (summary.isError || !summary.data) return <Alert tone="error" title="工作概览暂时不可用">
    <Button onClick={() => { void summary.refetch(); }}>重试</Button>
  </Alert>;

  const data = summary.data;
  return <section aria-labelledby="dashboard-title" className="grid gap-5">
    <h1 id="dashboard-title">工作概览</h1>
    <dl className="grid grid-cols-2 gap-4 rounded-panel border border-line bg-surface p-4 @min-[34rem]/workspace:grid-cols-4">
      <Count label="今日到期" value={data.counts.tasks_due_today} />
      <Count label="未完成事项" value={data.counts.tasks_open} />
      <Count label="活跃项目" value={data.counts.projects_active} />
      <Count label="失败任务" value={data.job_failures} />
    </dl>
    <div className="grid gap-4 @min-[54rem]/workspace:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <section aria-labelledby="dashboard-due" className="min-w-0">
        <Card title={<h2 id="dashboard-due" className="section-title">到期与逾期事项</h2>}>
          {data.due_tasks.length === 0 ? <Empty title="暂无到期事项" /> :
            <List aria-label="到期事项">{data.due_tasks.map(task => {
              const boundary = task.due ? dateBoundary(task.due, true) : null;
              const overdue = task.status !== "done" && task.status !== "cancelled" && boundary !== null && summary.dataUpdatedAt >= boundary;
              return <ListItem key={task.id} description={task.due ? `${task.due.date}${task.due.time ? ` ${task.due.time}` : ""}（上海）` : undefined}>
                <div className="flex flex-wrap items-center gap-2">
                  <Button variant="ghost" aria-label={`打开事项 ${task.title}`} className="page-link h-auto max-w-full !whitespace-normal !p-0 text-start wrap-anywhere" onClick={() => onSelectTask(task.id)}>{task.title}</Button>
                  <Badge tone={overdue ? "red" : "neutral"}>{overdue ? "已逾期" : "今日到期"}</Badge>
                </div>
              </ListItem>;
            })}</List>}
        </Card>
      </section>
      <div className="grid min-w-0 content-start gap-4">
        <Card title={<h2 className="section-title">最近笔记</h2>}>
          {data.recent_notes.length === 0 ? <Empty title="暂无最近笔记" /> :
            <List aria-label="最近笔记">{data.recent_notes.map(note => <ListItem key={note.id}>
              <Button variant="ghost" aria-label={`打开笔记 ${note.title}`} className="page-link h-auto max-w-full !whitespace-normal !p-0 text-start wrap-anywhere" onClick={() => onSelectNote(note.id)}>{note.title}</Button>
            </ListItem>)}</List>}
        </Card>
        <Card title={<h2 className="section-title">最近事项</h2>}>
          {data.recent_tasks.length === 0 ? <Empty title="暂无最近事项" /> :
            <List aria-label="最近事项">{data.recent_tasks.map(task => <ListItem key={task.id}>
              <Button variant="ghost" aria-label={`打开事项 ${task.title}`} className="page-link h-auto max-w-full !whitespace-normal !p-0 text-start wrap-anywhere" onClick={() => onSelectTask(task.id)}>{task.title}</Button>
            </ListItem>)}</List>}
        </Card>
      </div>
    </div>
    <Card title={<h2 className="section-title">对象统计</h2>}>
      <dl className="grid grid-cols-2 gap-4 @min-[40rem]/workspace:grid-cols-3 @min-[54rem]/workspace:grid-cols-6">
        <Count label="笔记" value={data.counts.notes} />
        <Count label="文件" value={data.counts.files} />
        <Count label="待办" value={data.tasks_by_status.todo} />
        <Count label="进行中" value={data.tasks_by_status.in_progress} />
        <Count label="已完成" value={data.tasks_by_status.done} />
        <Count label="已取消" value={data.tasks_by_status.cancelled} />
      </dl>
    </Card>
  </section>;
}
