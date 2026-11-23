import { useEffect, useReducer, useRef, useState, type FormEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert, Button, ConfirmDialog, Dialog, Empty, Form, Input, Link,
  List, ListItem, PageLayout, Paragraph, Popover, RadioGroup, Spinner, Tag,
} from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { TaskList } from "../tasks/TaskList";
import { projectQuery, projectsQuery, unfinishedTasksQuery, type Project } from "./queries";
import { ProjectAssociations } from "./ProjectAssociations";

const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试。";

function conflictMessage(cause: unknown, pending: string) {
  if (cause instanceof ApiRequestError && cause.status === 409) {
    return cause.currentVersion === undefined
      ? `${pending}（项目状态已变化，请刷新后重试）。`
      : `${pending}（当前版本 ${cause.currentVersion}）。`;
  }
  return message(cause);
}

export interface ProjectsPageProps {
  bundle: QueryClientBundle;
  projectId?: string;
  onSelectProject: (id: string | null) => void;
  onSelectTask?: (id: string) => void;
}

/** The parent route preserves the collection state while a detail is open. */
export function ProjectsPage({ bundle, projectId, onSelectProject, onSelectTask }: ProjectsPageProps) {
  const queryClient = useQueryClient();
  const [archivedFilter, setArchivedFilter] = useState(false);
  const [lastProject, setLastProject] = useState<string>();
  const restorePending = useRef(false);
  const returnLink = useRef<HTMLAnchorElement>(null);
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("active"));
  const projects = useQuery({
    ...projectsQuery(bundle, { archived: archivedFilter, cursor: page.cursor }),
    enabled: !projectId,
  });

  useEffect(() => {
    if (projectId || !restorePending.current || !projects.data) return;
    if (returnLink.current) {
      returnLink.current.focus({ preventScroll: true });
      returnLink.current.scrollIntoView?.({ block: "nearest" });
    }
    restorePending.current = false;
  }, [projectId, projects.data]);

  const invalidate = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }),
  ]);

  if (projectId) {
    return <ProjectDetail key={projectId} bundle={bundle} projectId={projectId}
      invalidate={invalidate} onDeleted={() => onSelectProject(null)} onSelectTask={onSelectTask} />;
  }

  const loading = projects.isFetching;
  const nextCursor = projects.data?.next_cursor ?? null;
  return (
    <section aria-label="项目列表">
      <PageLayout
        title="项目"
        actions={
          <ProjectCreate bundle={bundle} onCreated={async (id) => {
            await invalidate();
            onSelectProject(id);
          }} />
        }
      >
        <div className="space-y-4">
          <RadioGroup variant="segmented" label="归档筛选" value={archivedFilter ? "archived" : "active"}
            options={[{ value: "active", label: "未归档" }, { value: "archived", label: "已归档" }]}
            onValueChange={(value) => {
              setArchivedFilter(value === "archived");
              dispatch({ type: "filter", filterKey: value });
            }} />
          {loading && <Spinner label="正在读取项目" />}
          {projects.error && <Alert tone="error">
            {message(projects.error)}
            <Button disabled={loading} onClick={() => void projects.refetch()}>重试读取项目</Button>
          </Alert>}
          {!!projects.data?.items.length && <List aria-label="项目列表项">
            {projects.data.items.map((project) => (
              <ListItem key={project.id} description={project.description === null ? undefined :
                <span className="line-clamp-2 wrap-anywhere">{project.description}</span>}>
                <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                  <Link ref={lastProject === project.id ? returnLink : undefined} href={`/projects/${project.id}`} className="page-link wrap-anywhere" onClick={(event) => {
                    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
                    event.preventDefault();
                    restorePending.current = true;
                    setLastProject(project.id);
                    onSelectProject(project.id);
                  }}>{project.title}</Link>
                  {project.archived_at !== null && <Tag tone="neutral">已归档</Tag>}
                </div>
              </ListItem>
            ))}
          </List>}
          {projects.isSuccess && projects.data.items.length === 0 &&
            <Empty title={archivedFilter ? "没有已归档项目" : "还没有项目"} />}
          {(projects.data || page.previous.length > 0) && <nav aria-label="项目分页" className="flex flex-wrap gap-2">
            <Button disabled={loading || page.previous.length === 0}
              onClick={() => dispatch({ type: "previous" })}>上一页项目</Button>
            <Button disabled={loading || nextCursor === null || nextCursor === page.cursor}
              onClick={() => dispatch({ type: "next", nextCursor })}>下一页项目</Button>
          </nav>}
        </div>
      </PageLayout>
    </section>
  );
}

function ProjectCreate({ bundle, onCreated }: {
  bundle: QueryClientBundle;
  onCreated: (id: string) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState("");
  function updateOpen(next: boolean) {
    if (creating) return;
    setOpen(next);
    if (!next) {
      setTitle("");
      setDescription("");
      setError("");
    }
  }
  async function create(event: FormEvent) {
    event.preventDefault();
    if (creating) return;
    const trimmed = title.trim();
    if (!trimmed) {
      setError("请输入项目名称");
      return;
    }
    setCreating(true);
    setError("");
    try {
      const { data } = await bundle.api.raw.POST("/projects", {
        body: { title: trimmed, description: description.trim() || null },
      });
      if (!data) throw new Error("无法创建项目");
      await onCreated(data.id);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setCreating(false);
    }
  }
  return <Dialog title="新建项目" open={open} onOpenChange={updateOpen}
    trigger={<Button variant="blue">新建项目</Button>}>
    <Form onSubmit={(event) => void create(event)} className="gap-3 pb-4">
      <Form.Item label="新项目名称">
        <Input value={title} maxLength={500} onChange={(event) => setTitle(event.target.value)} disabled={creating} />
      </Form.Item>
      <Form.Item label="新项目说明">
        <Input.TextArea value={description} rows={3} onChange={(event) => setDescription(event.target.value)} disabled={creating} />
      </Form.Item>
      {error && <Alert tone="error">{error}</Alert>}
      <div className="flex flex-wrap justify-end gap-2">
        <Button disabled={creating} onClick={() => updateOpen(false)}>取消</Button>
        <Button type="submit" variant="blue" loading={creating}>创建项目</Button>
      </div>
    </Form>
  </Dialog>;
}

function ProjectDetail({ bundle, projectId, invalidate, onDeleted, onSelectTask }: {
  bundle: QueryClientBundle;
  projectId: string;
  invalidate: () => Promise<unknown>;
  onDeleted: () => void;
  onSelectTask?: (id: string) => void;
}) {
  const project = useQuery(projectQuery(bundle, projectId));
  const loaded = project.data;
  const [actionError, setActionError] = useState("");
  const [deleteError, setDeleteError] = useState("");
  const [busy, setBusy] = useState(false);
  const [showUnfinished, setShowUnfinished] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const menuRef = useRef<HTMLButtonElement>(null);
  const unfinished = useQuery({ ...unfinishedTasksQuery(bundle, projectId), enabled: showUnfinished });

  async function archive(action: "archive" | "unarchive") {
    if (!loaded || busy) return;
    setBusy(true);
    setActionError("");
    setShowUnfinished(false);
    const options = {
      params: { path: { projectId } },
      body: { expected_version: loaded.version },
    };
    try {
      const { data } = action === "archive"
        ? await bundle.api.raw.POST("/projects/{projectId}/archive", options)
        : await bundle.api.raw.POST("/projects/{projectId}/unarchive", options);
      if (data) bundle.queryClient.setQueryData(queryKeys.projects.detail(projectId), data);
      await invalidate();
    } catch (cause) {
      setActionError(conflictMessage(cause, action === "archive"
        ? "归档失败：仍有未完成事项，或项目已在别处修改"
        : "取消归档失败：项目状态已变化，请刷新后重试"));
      if (cause instanceof ApiRequestError && cause.status === 409) {
        await project.refetch();
        if (action === "archive") setShowUnfinished(true);
      }
    } finally {
      setBusy(false);
    }
  }

  if (!loaded && project.isPending) return <Spinner label="正在读取项目" />;
  if (!loaded) return <Alert tone="error">
    {project.error ? message(project.error) : "无法读取项目"}
    <Button disabled={project.isFetching} onClick={() => void project.refetch()}>重试读取项目</Button>
  </Alert>;

  const archived = loaded.archived_at !== null;
  const locked = busy || project.isFetching || project.isError;

  return (
    <section aria-label="项目详情">
      <PageLayout
        title={<span className="wrap-anywhere">{loaded.title}</span>}
        subtitle={
          <span className="inline-flex flex-wrap items-center gap-2">
            <span>版本 {loaded.version}</span>
            {archived && <Tag tone="neutral">已归档</Tag>}
          </span>
        }
        actions={
          <>
            <ProjectEdit bundle={bundle} project={loaded} disabled={archived || locked}
              invalidate={invalidate} refresh={async () => {
                const result = await project.refetch();
                if (result.error || !result.data) throw result.error ?? new Error("无法读取项目");
                return result.data;
              }} />
            {archived ? <Button disabled={locked} onClick={() => void archive("unarchive")}>取消归档</Button> :
              <Popover.Menu trigger={<Button ref={menuRef} disabled={locked}>更多项目操作</Button>} items={[
                { key: "archive", label: "归档项目", onSelect: () => void archive("archive") },
                { key: "delete", label: "删除项目", danger: true, onSelect: () => setDeleteOpen(true) },
              ]} />}
          </>
        }
      >
        <div className="space-y-5">
          {archived && <Alert tone="info">项目已归档，取消归档后可修改。</Alert>}
          {project.error && <Alert tone="error">
            {message(project.error)}
            <Button disabled={project.isFetching} onClick={() => void project.refetch()}>重试读取项目</Button>
          </Alert>}
          {actionError && <Alert tone="error">{actionError}</Alert>}
          {showUnfinished && unfinished.isFetching && <Spinner label="正在读取未完成事项" />}
          {showUnfinished && unfinished.error && <Alert tone="error">
            {message(unfinished.error)}
            <Button disabled={unfinished.isFetching} onClick={() => void unfinished.refetch()}>重试读取未完成事项</Button>
          </Alert>}
          {showUnfinished && !!unfinished.data?.length && (
            <Alert tone="warning" title="无法归档：仍有未完成事项">
              <p>完成、取消或移出下列事项后再归档。</p>
              <TaskList items={unfinished.data} projects={[loaded]} onSelectTask={onSelectTask} label="未完成事项" />
            </Alert>
          )}
          {loaded.description && <Paragraph className="reading-measure wrap-anywhere whitespace-pre-wrap">{loaded.description}</Paragraph>}
          <ProjectAssociations key={projectId} projectId={projectId} api={bundle.api} readOnly={archived || project.isError} />
        </div>
      </PageLayout>
      <ConfirmDialog
        open={deleteOpen}
        onOpenChange={(open) => {
          if (busy) return;
          setDeleteOpen(open);
          if (!open) {
            setDeleteError("");
            requestAnimationFrame(() => menuRef.current?.focus());
          }
        }}
        title="删除项目？"
        description="项目进入回收站，删除满 30 天后具备清理资格。项目下须无事项；笔记和资料不会被删除。"
        confirmText="确认删除项目"
        cancelText="取消"
        danger
        onConfirm={async () => {
          setBusy(true);
          setDeleteError("");
          try {
            await bundle.api.raw.DELETE("/projects/{projectId}", {
              params: { path: { projectId }, query: { expected_version: loaded.version } },
            });
            await invalidate();
            onDeleted();
          } catch (cause) {
            setDeleteError(conflictMessage(cause, "删除失败：项目下仍有事项，或项目已在别处修改；请先移出或删除事项后重试"));
            if (cause instanceof ApiRequestError && cause.status === 409) await project.refetch();
            throw cause;
          } finally {
            setBusy(false);
          }
        }}>
        {deleteError && <Alert tone="error">{deleteError}</Alert>}
      </ConfirmDialog>
    </section>
  );
}

function ProjectEdit({ bundle, project, disabled, invalidate, refresh }: {
  bundle: QueryClientBundle;
  project: Project;
  disabled: boolean;
  invalidate: () => Promise<unknown>;
  refresh: () => Promise<Project>;
}) {
  const [edit, setEdit] = useState<{ original: Project; version: number; title: string; description: string }>();
  const [current, setCurrent] = useState<Project>();
  const [refreshRequired, setRefreshRequired] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  function updateOpen(open: boolean) {
    if (busy) return;
    setError("");
    setCurrent(undefined);
    setRefreshRequired(false);
    setEdit(open
      ? { original: project, version: project.version, title: project.title, description: project.description ?? "" }
      : undefined);
  }
  async function refreshVersion() {
    try {
      const latest = await refresh();
      setCurrent(latest);
      setEdit((draft) => draft && { ...draft, version: latest.version });
      setRefreshRequired(false);
    } catch {
      setRefreshRequired(true);
    }
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!edit || busy || disabled || refreshRequired || current?.archived_at) return;
    if (!edit.title.trim()) {
      setError("项目名称不能为空");
      return;
    }
    const body: components["schemas"]["ProjectUpdate"] = { expected_version: edit.version };
    if (edit.title !== edit.original.title) body.title = edit.title.trim();
    const nextDescription = edit.description.trim() ? edit.description : null;
    if (nextDescription !== edit.original.description) body.description = nextDescription;
    if (body.title === undefined && body.description === undefined) {
      setError("没有需要保存的修改");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const { data } = await bundle.api.raw.PATCH("/projects/{projectId}", {
        params: { path: { projectId: project.id } }, body,
      });
      if (data) bundle.queryClient.setQueryData(queryKeys.projects.detail(project.id), data);
      await invalidate();
      setEdit(undefined);
    } catch (cause) {
      setError(conflictMessage(cause, "保存失败：项目已在别处修改，草稿已保留，请核对后重试"));
      if (cause instanceof ApiRequestError && cause.status === 409) await refreshVersion();
    } finally {
      setBusy(false);
    }
  }
  return <Dialog title="编辑项目" open={!!edit} onOpenChange={updateOpen}
    trigger={<Button variant="blue" disabled={disabled}>编辑项目</Button>}>
    {edit && <Form onSubmit={(event) => void save(event)} className="gap-3 pb-4">
      <Form.Item label="项目名称">
        <Input value={edit.title} maxLength={500} disabled={busy}
          onChange={(event) => setEdit({ ...edit, title: event.target.value })} />
      </Form.Item>
      <Form.Item label="项目说明">
        <Input.TextArea value={edit.description} rows={5} disabled={busy}
          onChange={(event) => setEdit({ ...edit, description: event.target.value })} />
      </Form.Item>
      {error && <Alert tone="error">{error}</Alert>}
      {current && <Alert tone="info" title={`服务端版本 ${current.version}`}>
        <p className="wrap-anywhere">{current.title}</p>
        {current.description && <p className="wrap-anywhere whitespace-pre-wrap">{current.description}</p>}
        {current.archived_at && <p>项目已归档，当前无法保存。</p>}
      </Alert>}
      {refreshRequired && <Alert tone="error">无法读取服务端版本。
        <Button disabled={busy} onClick={() => {
          setBusy(true);
          void refreshVersion().finally(() => setBusy(false));
        }}>重试读取服务端版本</Button>
      </Alert>}
      <div className="flex flex-wrap justify-end gap-2">
        <Button disabled={busy} onClick={() => updateOpen(false)}>取消</Button>
        <Button type="submit" variant="blue" loading={busy}
          disabled={disabled || refreshRequired || !!current?.archived_at}>保存修改</Button>
      </div>
    </Form>}
  </Dialog>;
}
