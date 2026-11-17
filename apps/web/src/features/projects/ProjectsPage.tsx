import { useReducer, useState, type FormEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  Badge,
  Button,
  ConfirmDialog,
  Empty,
  FormField,
  Input,
  List,
  ListItem,
  Select,
  Spinner,
  Textarea,
} from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { projectQuery, projectsQuery, unfinishedTasksQuery, type TaskStatus } from "./queries";
import { ProjectAssociations } from "./ProjectAssociations";

const STATUS_LABEL: Record<TaskStatus, string> = {
  todo: "待开始",
  in_progress: "进行中",
  done: "已完成",
  cancelled: "已取消",
};

const message = (cause: unknown) => (cause instanceof Error ? cause.message : "操作失败，请重试。");

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
  /** Selected project; the route adapter owns the URL and passes it down. */
  projectId?: string;
  onSelectProject: (id: string | null) => void;
}

/** Project list, creation, detail and lifecycle; belongs to the /projects route. */
export function ProjectsPage({ bundle, projectId, onSelectProject }: ProjectsPageProps) {
  const queryClient = useQueryClient();
  const [archivedFilter, setArchivedFilter] = useState(false);
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("active"));
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState("");

  const projects = useQuery(projectsQuery(bundle, { archived: archivedFilter, cursor: page.cursor }));
  const loadingList = projects.isFetching;

  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() }),
      // Archiving also changes the tasks of this project.
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }),
    ]);

  function changeFilter(next: boolean) {
    setArchivedFilter(next);
    dispatch({ type: "filter", filterKey: next ? "archived" : "active" });
  }

  async function create(event: FormEvent) {
    event.preventDefault();
    if (creating) return;
    const trimmed = title.trim();
    if (!trimmed) {
      setCreateError("请输入项目名称");
      return;
    }
    setCreating(true);
    setCreateError("");
    try {
      const { data } = await bundle.api.raw.POST("/projects", {
        body: {
          title: trimmed,
          description: description.trim() ? description.trim() : null,
        },
      });
      if (!data) throw new Error("无法创建项目");
      setTitle("");
      setDescription("");
      await invalidate();
      onSelectProject(data.id);
    } catch (cause) {
      setCreateError(message(cause));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="space-y-6">
      <section aria-label="项目列表" className="space-y-3">
        <h2 className="text-lg font-semibold">项目</h2>
        <form onSubmit={(event) => void create(event)} className="flex flex-wrap items-end gap-2">
          <FormField label="新项目名称">
            <Input value={title} maxLength={500} onChange={(event) => setTitle(event.target.value)} disabled={creating} />
          </FormField>
          <FormField label="新项目说明">
            <Textarea value={description} rows={2} onChange={(event) => setDescription(event.target.value)} disabled={creating} />
          </FormField>
          <Button type="submit" disabled={creating}>
            创建项目
          </Button>
        </form>
        {createError && <Alert tone="error">{createError}</Alert>}
        <div className="flex flex-wrap items-end gap-2">
          <FormField label="归档筛选">
            <Select
              value={archivedFilter ? "archived" : "active"}
              onValueChange={(value) => {
                if (value) changeFilter(value === "archived");
              }}
              options={[
                { value: "active", label: "未归档" },
                { value: "archived", label: "已归档" },
              ]}
            />
          </FormField>
          <Button
            disabled={loadingList || page.previous.length === 0}
            onClick={() => dispatch({ type: "previous" })}
          >
            上一页项目
          </Button>
          <Button
            disabled={loadingList || !projects.data?.next_cursor}
            onClick={() => dispatch({ type: "next", nextCursor: projects.data?.next_cursor ?? null })}
          >
            下一页项目
          </Button>
          {loadingList && <Spinner label="正在读取项目" />}
        </div>
        {projects.error && (
          <Alert tone="error">
            {projects.error instanceof Error ? projects.error.message : "无法读取项目列表"}
            <Button
              disabled={loadingList}
              onClick={() => {
                void projects.refetch();
              }}
            >
              重试读取项目
            </Button>
          </Alert>
        )}
        <List aria-label="项目列表项">
          {projects.data?.items.map((project) => (
            <ListItem
              key={project.id}
              description={project.description ?? undefined}
              actions={
                <Button aria-label={`打开项目 ${project.title}`} onClick={() => onSelectProject(project.id)}>
                  打开
                </Button>
              }
            >
              {project.title}
              {project.archived_at !== null && <Badge tone="neutral">已归档</Badge>}
            </ListItem>
          ))}
        </List>
        {projects.data?.items.length === 0 && (
          <Empty
            title={archivedFilter ? "没有已归档项目" : "还没有项目"}
            description={archivedFilter ? "归档项目后可以在这里找到并取消归档。" : "创建第一个项目，把相关笔记、文件和事项组织在一起。"}
          />
        )}
      </section>
      {projectId ? (
        <ProjectDetail key={projectId} bundle={bundle} projectId={projectId} onDeleted={() => onSelectProject(null)} />
      ) : (
        <Empty title="未选择项目" description="从上面的列表打开一个项目，或先创建项目。" />
      )}
    </div>
  );
}

function ProjectDetail({
  bundle,
  projectId,
  onDeleted,
}: {
  bundle: QueryClientBundle;
  projectId: string;
  onDeleted: () => void;
}) {
  const queryClient = useQueryClient();
  const project = useQuery(projectQuery(bundle, projectId));
  const loaded = project.data;
  const [draft, setDraft] = useState<{ title: string; description: string } | null>(null);
  const [saveError, setSaveError] = useState("");
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState("");
  const [busy, setBusy] = useState(false);
  const [showUnfinished, setShowUnfinished] = useState(false);

  // The detail is keyed by project id, so one initialization per mount keeps
  // local edits while a refetch (or another tab's save) updates the server copy.
  if (loaded && draft === null) {
    setDraft({ title: loaded.title, description: loaded.description ?? "" });
  }

  const unfinished = useQuery({
    ...unfinishedTasksQuery(bundle, projectId),
    enabled: showUnfinished,
  });

  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() }),
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }),
    ]);

  async function save() {
    if (!loaded || !draft || saving) return;
    if (!draft.title.trim()) {
      setSaveError("项目名称不能为空");
      return;
    }
    const body: components["schemas"]["ProjectUpdate"] = { expected_version: loaded.version };
    if (draft.title !== loaded.title) body.title = draft.title;
    const nextDescription = draft.description.trim() === "" ? null : draft.description;
    if (nextDescription !== loaded.description) body.description = nextDescription;
    if (body.title === undefined && body.description === undefined) {
      setSaveError("没有需要保存的修改");
      return;
    }
    setSaving(true);
    setSaveError("");
    try {
      await bundle.api.raw.PATCH("/projects/{projectId}", { params: { path: { projectId } }, body });
      await invalidate();
    } catch (cause) {
      setSaveError(conflictMessage(cause, "保存失败：项目已在别处修改，草稿已保留，请核对后重试"));
    } finally {
      setSaving(false);
    }
  }

  async function archive() {
    if (!loaded || busy) return;
    setBusy(true);
    setActionError("");
    try {
      await bundle.api.raw.POST("/projects/{projectId}/archive", {
        params: { path: { projectId } },
        body: { expected_version: loaded.version },
      });
      setShowUnfinished(false);
      await invalidate();
    } catch (cause) {
      setActionError(conflictMessage(cause, "归档失败：仍有未完成事项，或项目已在别处修改"));
      if (cause instanceof ApiRequestError && cause.status === 409) {
        setShowUnfinished(true);
      }
    } finally {
      setBusy(false);
    }
  }

  async function unarchive() {
    if (!loaded || busy) return;
    setBusy(true);
    setActionError("");
    try {
      await bundle.api.raw.POST("/projects/{projectId}/unarchive", {
        params: { path: { projectId } },
        body: { expected_version: loaded.version },
      });
      await invalidate();
    } catch (cause) {
      setActionError(conflictMessage(cause, "取消归档失败：项目状态已变化，请刷新后重试"));
    } finally {
      setBusy(false);
    }
  }

  if (project.isPending) {
    return <Spinner label="正在读取项目" />;
  }
  if (project.error || !loaded) {
    return (
      <Alert tone="error">{project.error instanceof Error ? project.error.message : "无法读取项目"}</Alert>
    );
  }

  const archived = loaded.archived_at !== null;

  return (
    <section aria-label="项目详情" className="space-y-4">
      <header className="flex flex-wrap items-center gap-2">
        <h2 className="text-lg font-semibold">{loaded.title}</h2>
        {archived && <Badge tone="neutral">已归档</Badge>}
        <span className="text-sm text-muted">版本 {loaded.version}</span>
      </header>
      {archived && <Alert tone="info">已归档项目为只读；取消归档后才能编辑、关联或删除。</Alert>}
      {actionError && <Alert tone="error">{actionError}</Alert>}
      {showUnfinished && unfinished.isFetching && <Spinner label="正在读取未完成事项" />}
      {unfinished.error && (
        <Alert tone="error">
          {unfinished.error instanceof Error ? unfinished.error.message : "无法读取未完成事项"}
        </Alert>
      )}
      {(unfinished.data?.length ?? 0) > 0 && (
        <Alert tone="warning" title="无法归档：仍有未完成事项">
          <p>完成或取消下列事项，或先把它们移出本项目，然后再归档。</p>
          <List aria-label="未完成事项">
            {unfinished.data?.map((task) => (
              <ListItem key={task.id} description={STATUS_LABEL[task.status]}>
                {task.title}
              </ListItem>
            ))}
          </List>
        </Alert>
      )}

      <form
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
        className="space-y-2"
      >
        <FormField label="项目名称">
          <Input
            value={draft?.title ?? ""}
            maxLength={500}
            disabled={archived || saving}
            onChange={(event) => setDraft((current) => (current ? { ...current, title: event.target.value } : current))}
          />
        </FormField>
        <FormField label="项目说明">
          <Textarea
            value={draft?.description ?? ""}
            rows={3}
            disabled={archived || saving}
            onChange={(event) => setDraft((current) => (current ? { ...current, description: event.target.value } : current))}
          />
        </FormField>
        <div className="flex flex-wrap gap-2">
          <Button type="submit" disabled={archived || saving}>
            保存修改
          </Button>
          {archived ? (
            <Button disabled={busy} onClick={() => void unarchive()}>
              取消归档
            </Button>
          ) : (
            <Button disabled={busy} onClick={() => void archive()}>
              归档项目
            </Button>
          )}
          <ConfirmDialog
            title="删除项目？"
            description="删除后项目进入回收站，30 天内可恢复。项目中已归档的事项会保留，笔记和资料不会被删除。"
            confirmText="确认删除项目"
            cancelText="取消"
            danger
            children={actionError ? <Alert tone="error">{actionError}</Alert> : undefined}
            trigger={
              <Button danger disabled={archived || busy}>
                删除项目
              </Button>
            }
            onConfirm={async () => {
              setActionError("");
              try {
                await bundle.api.raw.DELETE("/projects/{projectId}", {
                  params: { path: { projectId }, query: { expected_version: loaded.version } },
                });
                await invalidate();
                onDeleted();
              } catch (cause) {
                setActionError(conflictMessage(cause, "删除失败：项目下仍有事项，或项目已在别处修改；请先移出或删除事项后重试"));
                throw cause;
              }
            }}
          />
        </div>
        {saveError && <Alert tone="error">{saveError}</Alert>}
      </form>

      <ProjectAssociations projectId={projectId} api={bundle.api} readOnly={archived} />
    </section>
  );
}
