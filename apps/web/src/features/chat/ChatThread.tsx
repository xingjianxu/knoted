import { Alert, Badge, Button, Checkbox, Empty, Form, FormField, MultiSelect, Spinner, Textarea } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import type { QueryClientBundle } from "../../lib/query";
import { searchTagChoicesQuery } from "../search/queries";
import { projectChoicesQuery } from "../tasks/queries";
import { CitationList, CitationPanel } from "./CitationPanel";
import { runQuery, sessionQuery, type ChatMessage, type ChatRequest, type ChatRun, type ChatScope } from "./queries";
import { runIsActive, useChatRun, viewRun, type ViewedRun } from "./use-run";

const sourceOptions = [
  { value: "note", label: "笔记" },
  { value: "file", label: "文件" },
  { value: "task", label: "事项" },
] as const;

function stateLabel({ run, disconnected, stopping }: ViewedRun) {
  if (stopping) return "正在停止";
  if (disconnected && runIsActive(run)) return "连接中断";
  switch (run.state) {
    case "queued": return "排队中";
    case "started": return "生成中";
    case "completed": return run.partial ? "生成中断" : "已完成";
    case "cancelled": return "已停止";
    case "failed": return run.error === "interrupted" ? "生成中断" : "生成失败";
  }
}

export function StreamMessage({ view, actions, busy }: {
  view: ViewedRun;
  actions: {
    stop: (run: ChatRun) => Promise<boolean>;
    retry: (run: ChatRun) => Promise<boolean>;
    refresh: (run: ChatRun) => Promise<boolean>;
  };
  busy: boolean;
}) {
  const { run } = view;
  const active = runIsActive(run);
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2" aria-live="polite">
        <Badge tone={run.state === "failed" || view.disconnected ? "orange" : "neutral"}>{stateLabel(view)}</Badge>
        {run.partial && !active && <Badge tone="orange">未完成回答</Badge>}
      </div>
      {view.body && <p className="whitespace-pre-wrap wrap-anywhere">{view.body}</p>}
      {view.citations.length > 0 && <CitationList citations={view.citations} />}
      <div className="flex flex-wrap gap-2">
        {active && <Button disabled={busy || view.stopping} onClick={() => void actions.stop(run)}>停止生成</Button>}
        {active && (view.disconnected || view.stopping) && (
          <Button disabled={busy} onClick={() => void actions.refresh(run)}>查询状态</Button>
        )}
        {(run.state === "failed" || run.state === "cancelled") && (
          <Button disabled={busy} onClick={() => void actions.retry(run)}>重试生成</Button>
        )}
      </div>
    </div>
  );
}

function QuestionForm({ bundle, provider, blocked, onSubmit }: {
  bundle: QueryClientBundle;
  provider: string;
  blocked: boolean;
  onSubmit: (body: ChatRequest) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState("");
  const [types, setTypes] = useState<string[]>([]);
  const [projects, setProjects] = useState<string[]>([]);
  const [tags, setTags] = useState<string[]>([]);
  const projectOptions = useQuery(projectChoicesQuery(bundle));
  const tagOptions = useQuery(searchTagChoicesQuery(bundle));
  return (
    <Form className="space-y-3" onSubmit={(event) => {
      event.preventDefault();
      const content = draft.trim();
      if (!content || blocked) return;
      const scope: ChatScope = {
        types: types.length ? sourceOptions.filter((source) => types.includes(source.value)).map((source) => source.value) : undefined,
        project_ids: projects.length ? [...projects].sort() : undefined,
        tag_ids: tags.length ? [...tags].sort() : undefined,
      };
      void onSubmit({ content, scope }).then((accepted) => {
        if (accepted) setDraft((current) => current === draft ? "" : current);
      });
    }}>
      <FormField label="问题">
        <Textarea value={draft} onChange={(event) => setDraft(event.target.value)} maxLength={20_000} rows={3} />
      </FormField>
      <div className="grid gap-2 sm:grid-cols-3">
        <fieldset className="space-y-1">
          <legend className="text-sm font-medium">资料类型</legend>
          <div className="flex flex-wrap gap-x-2">
            {sourceOptions.map(({ value, label }) => (
              <Checkbox
                key={value}
                label={label}
                checked={types.includes(value)}
                onCheckedChange={(checked) => setTypes((selected) => (
                  checked ? [...selected, value] : selected.filter((item) => item !== value)
                ))}
              />
            ))}
          </div>
        </fieldset>
        <FormField label="项目范围">
          <MultiSelect
            options={(projectOptions.data ?? []).map((project) => ({ value: project.id, label: project.title }))}
            value={projects}
            onValueChange={setProjects}
          />
        </FormField>
        <FormField label="标签范围">
          <MultiSelect
            options={(tagOptions.data ?? []).map((tag) => ({ value: tag.id, label: tag.name }))}
            value={tags}
            onValueChange={setTags}
          />
        </FormField>
      </div>
      {(tagOptions.isError || projectOptions.isError) && <Alert tone="error">
        无法读取资料范围选项。{" "}
        <Button type="button" onClick={() => {
          if (tagOptions.isError) void tagOptions.refetch();
          if (projectOptions.isError) void projectOptions.refetch();
        }}>重试</Button>
      </Alert>}
      <div className="flex flex-wrap items-center justify-end gap-3">
        {provider !== "local" && <span className="text-sm text-muted">问题和相关资料将发送给外部服务 {provider}。</span>}
        <Button type="submit" variant="solid" disabled={blocked || !draft.trim()}>发送问题</Button>
      </div>
    </Form>
  );
}

function MessageRow({ message, bundle, selected, children }: {
  message: ChatMessage;
  bundle: QueryClientBundle;
  selected: boolean;
  children?: React.ReactNode;
}) {
  const element = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (!selected) return;
    element.current?.scrollIntoView?.({ block: "center" });
    element.current?.focus({ preventScroll: true });
  }, [selected]);
  return (
    <li
      className={`space-y-2 rounded-panel border p-4 ${selected ? "border-accent bg-accent-soft" : "border-line bg-surface"}`}
      ref={element}
      tabIndex={selected ? -1 : undefined}
      data-message-id={message.id}
    >
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{message.role === "user" ? "你" : "助手"}</span>
        {message.incomplete && !children && <Badge tone="orange">未完成回答</Badge>}
      </div>
      {children ?? <p className="whitespace-pre-wrap wrap-anywhere">{message.body}</p>}
      {message.role === "assistant" && !message.incomplete && <CitationPanel bundle={bundle} messageId={message.id} />}
    </li>
  );
}

export function ChatThread({ bundle, sessionId, targetMessage }: {
  bundle: QueryClientBundle;
  sessionId: string;
  targetMessage: string | undefined;
}) {
  const query = useQuery(sessionQuery(bundle, sessionId));
  const controls = useChatRun(bundle, sessionId);
  const session = query.data;
  const latest = session?.messages.at(-1);
  const persistedRunId = !controls.current && latest && (latest.role === "user" || latest.incomplete) ? latest.run_id : null;
  const persisted = useQuery(runQuery(bundle, sessionId, persistedRunId ?? null));
  const viewed = controls.current ?? (persisted.data ? { ...viewRun(persisted.data), disconnected: true } : undefined);
  const attached = viewed && session?.messages.find((message) => message.role === "assistant" &&
    (message.run_id === viewed.run.id || message.id === viewed.run.message_id));
  const showStream = viewed && (!attached || attached.incomplete);

  if (query.isPending) return <Spinner label="正在读取会话" />;
  if (query.isError || !session) {
    return <Alert tone="error">无法读取会话。{" "}<Button onClick={() => void query.refetch()}>重试</Button></Alert>;
  }
  return (
    <section className="min-w-0 space-y-4" aria-label="会话详情">
      <header className="flex flex-wrap items-center gap-2">
        <h2 className="text-xl font-semibold wrap-anywhere">{session.title}</h2>
        <Badge>{session.provider === "local" ? "本地服务" : `外部服务 ${session.provider}`}</Badge>
      </header>
      {targetMessage && !session.messages.some((item) => item.id === targetMessage) && (
        <Alert tone="warning">这条消息已不存在。</Alert>
      )}
      {session.messages.length === 0 && !showStream && <Empty title="开始问答" />}
      <ol className="space-y-3" aria-label="会话消息">
        {session.messages.map((message) => (
          <MessageRow key={message.id} message={message} selected={targetMessage === message.id} bundle={bundle}>
            {showStream && attached?.id === message.id ? (
              <StreamMessage view={{ ...viewed, body: viewed.body || message.body }} actions={controls} busy={controls.busy} />
            ) : undefined}
          </MessageRow>
        ))}
        {showStream && !attached && (
          <li className="space-y-2 rounded-panel border border-line bg-surface p-4">
            <span className="text-sm font-medium">助手</span>
            <StreamMessage view={viewed} actions={controls} busy={controls.busy} />
          </li>
        )}
      </ol>
      {controls.problem && <Alert tone="error">{controls.problem}</Alert>}
      {persisted.isError && !controls.current && (
        <Alert tone="error">无法读取运行状态。{" "}<Button onClick={() => void persisted.refetch()}>重试</Button></Alert>
      )}
      <QuestionForm
        bundle={bundle}
        provider={session.provider}
        blocked={controls.busy || (!!viewed && runIsActive(viewed.run))}
        onSubmit={controls.submit}
      />
    </section>
  );
}
