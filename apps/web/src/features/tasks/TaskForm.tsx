import { useState } from "react";
import { Controller, useForm, useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { Alert, Button, FormField, Input, Select, Textarea } from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import { dateBoundary } from "./dates";
import type { Project, Task } from "./queries";
export const statusOptions = [
  { value: "todo", label: "待开始" }, { value: "in_progress", label: "进行中" },
  { value: "done", label: "已完成" }, { value: "cancelled", label: "已取消" },
];
const schema = z.object({
  title: z.string().trim().min(1, "请输入事项标题").refine((value) => [...value].length <= 500 && !value.includes("\0"), "标题最多500字且不能含空字符"),
  description: z.string().refine((value) => !value.includes("\0"), "说明不能含空字符"),
  status: z.enum(["todo", "in_progress", "done", "cancelled"]),
  priority: z.enum(["none", "low", "medium", "high"]),
  project_id: z.string(), parent_id: z.string(),
  startDate: z.string(), startTime: z.string(), dueDate: z.string(), dueTime: z.string(),
}).superRefine((value, ctx) => {
  const boundaries: (number | null)[] = [];
  for (const kind of ["start", "due"] as const) {
    const date = value[`${kind}Date`]; const time = value[`${kind}Time`];
    if (!date) {
      boundaries.push(null);
      if (time) ctx.addIssue({ code: "custom", path: [`${kind}Date`], message: "填写时间前请选择日期" });
      continue;
    }
    const boundary = dateBoundary({ date, time: time || null, timezone: "Asia/Shanghai" }, kind === "due");
    boundaries.push(boundary);
    if (boundary === null) ctx.addIssue({ code: "custom", path: [`${kind}Date`], message: "日期无效，或上海本地时间不存在或重复" });
  }
  const [start, due] = boundaries;
  if (start != null && due != null && start > due) ctx.addIssue({ code: "custom", path: ["dueDate"], message: "开始不能晚于截止" });
});
type Draft = z.infer<typeof schema>;
function initial(task?: Task, parent?: Task): Draft {
  return { title: task?.title ?? "", description: task?.description ?? "", status: task?.status ?? "todo", priority: task?.priority ?? "none", project_id: task?.project_id ?? parent?.project_id ?? "", parent_id: task?.parent_id ?? parent?.id ?? "", startDate: task?.start?.date ?? "", startTime: task?.start?.time ?? "", dueDate: task?.due?.date ?? "", dueTime: task?.due?.time ?? "" };
}
export function TaskForm({ task, parent, projects, parents, disabled = false, pending = false, conflict, onSubmit, submitLabel }: {
  task?: Task; parent?: Task; projects: Project[]; parents: Task[]; disabled?: boolean; pending?: boolean;
  conflict?: boolean; onSubmit: (input: components["schemas"]["TaskCreate"]) => Promise<void>; submitLabel: string;
}) {
  const { register, control, handleSubmit, setError, reset, formState: { errors, isSubmitting } } = useForm<Draft>({ resolver: zodResolver(schema), defaultValues: initial(task, parent) });
  const [error, setMessage] = useState(""); const [copyMessage, setCopyMessage] = useState("");
  const draft = useWatch({ control }) as Draft;
  const parentId = draft.parent_id; const selectedParent = parent ?? parents.find((row) => row.id === parentId);
  const busy = pending || isSubmitting;
  const locked = disabled || busy;
  const input = (value: Draft): components["schemas"]["TaskCreate"] => ({
    title: value.title, description: value.description || null, status: value.status, priority: value.priority,
    parent_id: value.parent_id || null, project_id: selectedParent ? selectedParent.project_id : value.project_id || null,
    start: value.startDate ? { date: value.startDate, time: value.startTime || null, timezone: "Asia/Shanghai" } : null,
    due: value.dueDate ? { date: value.dueDate, time: value.dueTime || null, timezone: "Asia/Shanghai" } : null,
  });
  const copy = JSON.stringify(input(draft), null, 2);
  const submit = handleSubmit(async (value) => {
    setMessage("");
    try { await onSubmit(input(value)); if (!task) reset(initial(undefined, parent)); }
    catch (cause) {
      if (cause instanceof ApiRequestError) {
        for (const field of cause.fieldErrors ?? []) {
          const names: (keyof Draft)[] = field.field === "start/due" ? ["startDate"] : field.field === "start" ? ["startDate"] : field.field === "due" ? ["dueDate"] : field.field in value ? [field.field as keyof Draft] : [];
          for (const name of names) setError(name, { type: "server", message: field.message });
        }
      }
      setMessage(cause instanceof Error ? cause.message : "保存失败，请重试");
    }
  });
  return <form noValidate onSubmit={(event) => void submit(event)} className="space-y-3">
    <FormField label="事项标题" error={errors.title?.message}><Input {...register("title")} disabled={locked} /></FormField>
    <FormField label="事项说明" error={errors.description?.message}><Textarea {...register("description")} rows={3} disabled={locked} /></FormField>
    <div className="flex flex-wrap gap-3">
      <Controller control={control} name="status" render={({ field }) => <FormField label="事项状态" error={errors.status?.message}><Select value={field.value} onValueChange={field.onChange} options={statusOptions} disabled={locked} /></FormField>} />
      <Controller control={control} name="priority" render={({ field }) => <FormField label="优先级" error={errors.priority?.message}><Select value={field.value} onValueChange={field.onChange} options={[{ value: "none", label: "无" }, { value: "low", label: "低" }, { value: "medium", label: "中" }, { value: "high", label: "高" }]} disabled={locked} /></FormField>} />
      <Controller control={control} name="parent_id" render={({ field }) => <FormField label="父事项" error={errors.parent_id?.message}><Select value={field.value} onValueChange={field.onChange} options={[{ value: "", label: "顶层事项" }, ...parents.filter((row) => row.id !== task?.id).map((row) => ({ value: row.id, label: row.title })), ...(parent && !parents.some((row) => row.id === parent.id) ? [{ value: parent.id, label: parent.title }] : [])]} disabled={locked || !!parent} /></FormField>} />
      <Controller control={control} name="project_id" render={({ field }) => <FormField label="所属项目" error={errors.project_id?.message} description={parentId ? "子事项继承父事项所属项目" : undefined}><Select value={selectedParent ? selectedParent.project_id ?? "" : field.value} onValueChange={field.onChange} options={[{ value: "", label: "独立事项" }, ...projects.map((row) => ({ value: row.id, label: row.title, disabled: row.archived_at !== null }))]} disabled={locked || !!parentId} /></FormField>} />
    </div>
    <p className="text-sm text-muted">日期和时间均按 Asia/Shanghai；时间留空表示纯日期，截止于次日零点。</p>
    <div className="flex flex-wrap gap-3">
      <FormField label="开始日期" error={errors.startDate?.message}><Input type="date" {...register("startDate")} disabled={locked} /></FormField>
      <FormField label="开始时间" error={errors.startTime?.message}><Input type="time" step={60} {...register("startTime")} disabled={locked} /></FormField>
      <FormField label="截止日期" error={errors.dueDate?.message}><Input type="date" {...register("dueDate")} disabled={locked} /></FormField>
      <FormField label="截止时间" error={errors.dueTime?.message}><Input type="time" step={60} {...register("dueTime")} disabled={locked} /></FormField>
    </div>
    <Button type="submit" disabled={locked || conflict}>{submitLabel}</Button>
    {error && <Alert tone="error">{error}</Alert>}
    {conflict && <div className="space-y-2">
      <FormField label="草稿副本" description="可选中复制全部内容"><Textarea value={copy} readOnly rows={8} /></FormField>
      <Button type="button" onClick={() => { void navigator.clipboard?.writeText(copy).then(() => setCopyMessage("草稿已复制")).catch(() => setCopyMessage("复制失败，请从草稿副本手动复制")); }}>复制草稿</Button>
      {copyMessage && <p role="status">{copyMessage}</p>}
    </div>}
  </form>;
}
