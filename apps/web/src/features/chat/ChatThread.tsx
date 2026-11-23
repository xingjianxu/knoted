import { AiMessage, Alert, Button, Card, Checkbox, Divider, Fieldset, Form, Input, Select, Spinner, Tag, Text } from "@bixlab/mtone";
import { ApiRequestError } from "@knoted/api-client";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import type { QueryClientBundle } from "../../lib/query";
import { searchTagChoicesQuery } from "../search/queries";
import { projectChoicesQuery } from "../tasks/queries";
import { CitationList, CitationPanel } from "./CitationPanel";
import { SaveNoteDialog } from "./SaveNoteDialog";
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
    case "failed": return run.error === "generation_interrupted" ? "生成中断" : "生成失败";
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
  // A finished answer may have been rewritten by citation validation.
  const showDraft = run.state !== "completed" || run.partial;
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2" aria-live="polite">
        <Tag tone={run.state === "failed" || view.disconnected ? "orange" : "neutral"}>{stateLabel(view)}</Tag>
        {run.partial && !active && <Tag tone="orange">未完成回答</Tag>}
      </div>
      {showDraft && view.body && <p className="whitespace-pre-wrap wrap-anywhere leading-relaxed">{view.body}</p>}
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
  const [expanded, setExpanded] = useState(false);
  const scopeId = useId();
  const projectOptions = useQuery({ ...projectChoicesQuery(bundle), enabled: expanded });
  const tagOptions = useQuery({ ...searchTagChoicesQuery(bundle), enabled: expanded });
  const projectLabels = new Map((projectOptions.data ?? []).map(({ id, title }) => [id, title]));
  const tagLabels = new Map((tagOptions.data ?? []).map(({ id, name }) => [id, name]));
  for (const id of projects) {
    if (!projectLabels.has(id)) projectLabels.set(id, id);
  }
  for (const id of tags) {
    if (!tagLabels.has(id)) tagLabels.set(id, id);
  }
  const summary = [
    ...sourceOptions.filter(({ value }) => types.includes(value)).map(({ label }) => label),
    projects.length > 0 ? `${projects.length} 个项目` : null,
    tags.length > 0 ? `${tags.length} 个标签` : null,
  ].filter(Boolean).join(" · ");
  return (
    <div className="space-y-3">
      <Divider />
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
        <Form.Item label="问题">
          <Input.TextArea value={draft} onChange={(event) => setDraft(event.target.value)} maxLength={20_000} rows={3} />
        </Form.Item>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" aria-expanded={expanded} aria-controls={scopeId} onClick={() => setExpanded(!expanded)}>
            资料范围
          </Button>
          {!expanded && <Text muted className="text-sm">{summary || "全部资料"}</Text>}
        </div>
        <div id={scopeId} hidden={!expanded}>
          {expanded && (
            <Card size="small">
              <div className="grid gap-3 @min-[42rem]/workspace:grid-cols-2">
                <Fieldset legend="资料类型" className="min-w-0 @min-[42rem]/workspace:col-span-2">
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
                </Fieldset>
                <div className="min-w-0 space-y-2">
                  <Form.Item label="项目范围">
                    <Select
                      mode="multiple"
                      options={Array.from(projectLabels, ([value, label]) => ({ value, label }))}
                      value={projects}
                      onValueChange={setProjects}
                    />
                  </Form.Item>
                  {projectOptions.isPending && <Spinner label="正在读取项目选项" />}
                  {projectOptions.isError && <Alert tone="error">
                    无法读取项目选项。{" "}
                    <Button type="button" onClick={() => void projectOptions.refetch()}>重试读取项目</Button>
                  </Alert>}
                </div>
                <div className="min-w-0 space-y-2">
                  <Form.Item label="标签范围">
                    <Select
                      mode="multiple"
                      options={Array.from(tagLabels, ([value, label]) => ({ value, label }))}
                      value={tags}
                      onValueChange={setTags}
                    />
                  </Form.Item>
                  {tagOptions.isPending && <Spinner label="正在读取标签选项" />}
                  {tagOptions.isError && <Alert tone="error">
                    无法读取标签选项。{" "}
                    <Button type="button" onClick={() => void tagOptions.refetch()}>重试读取标签</Button>
                  </Alert>}
                </div>
              </div>
            </Card>
          )}
        </div>
        <div className="flex flex-wrap items-center justify-end gap-3">
          {provider !== "local" && <Text muted className="text-sm wrap-anywhere">问题和相关资料将发送给外部服务 {provider}。</Text>}
          <Button type="submit" variant="solid" disabled={blocked || !draft.trim()}>发送问题</Button>
        </div>
      </Form>
    </div>
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
    const target = element.current;
    if (!selected || !target) return;
    target.focus({ preventScroll: true });
    // A cached thread can render before the router resets the workspace scroll.
    const frame = requestAnimationFrame(() => target.scrollIntoView?.({ block: "center" }));
    return () => cancelAnimationFrame(frame);
  }, [selected]);
  return (
    <li
      className={selected ? "rounded-panel outline-2 outline-offset-2 outline-accent" : undefined}
      ref={element}
      tabIndex={selected ? -1 : undefined}
      data-message-id={message.id}
    >
      <AiMessage from={message.role === "user" ? "user" : "assistant"}>
        <div className="space-y-2">
          {message.incomplete && !children && <Tag tone="orange">未完成回答</Tag>}
          {children ?? <p className="whitespace-pre-wrap wrap-anywhere leading-relaxed">{message.body}</p>}
          {message.role === "assistant" && !message.incomplete && (
            <CitationPanel bundle={bundle} messageId={message.id}>
              <SaveNoteDialog bundle={bundle} messageId={message.id} />
            </CitationPanel>
          )}
        </div>
      </AiMessage>
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
  const unreadable = query.error instanceof ApiRequestError && [401, 403, 404].includes(query.error.status);
  if (!session || unreadable) {
    return <Alert tone="error">无法读取会话。{" "}<Button onClick={() => void query.refetch()}>重试</Button></Alert>;
  }
  return (
    <section className="reading-measure min-w-0 space-y-5" aria-label="会话详情">
      <header className="flex flex-wrap items-center gap-2">
        <h1 className="page-title">{session.title}</h1>
        <Tag className="max-w-full wrap-anywhere">{session.provider === "local" ? "本地服务" : `外部服务 ${session.provider}`}</Tag>
      </header>
      {query.isError && <Alert tone="error">会话刷新失败。{" "}<Button onClick={() => void query.refetch()}>重试刷新会话</Button></Alert>}
      {targetMessage && !session.messages.some((item) => item.id === targetMessage) && (
        <Alert tone="warning">这条消息已不存在。</Alert>
      )}
      {(session.messages.length > 0 || showStream) && <ol className="space-y-4" aria-label="会话消息">
        {session.messages.map((message) => (
          <MessageRow key={message.id} message={message} selected={targetMessage === message.id} bundle={bundle}>
            {showStream && attached?.id === message.id ? (
              <StreamMessage view={{ ...viewed, body: viewed.body || message.body }} actions={controls} busy={controls.busy} />
            ) : undefined}
          </MessageRow>
        ))}
        {showStream && !attached && (
          <li>
            <AiMessage from="assistant">
              <StreamMessage view={viewed} actions={controls} busy={controls.busy} />
            </AiMessage>
          </li>
        )}
      </ol>}
      {controls.problem && <Alert tone="error">{controls.problem}</Alert>}
      {persistedRunId && persisted.isPending && <Spinner label="正在读取运行状态" />}
      {persisted.isError && !controls.current && (
        <Alert tone="error">无法读取运行状态。{" "}<Button onClick={() => void persisted.refetch()}>重试</Button></Alert>
      )}
      <QuestionForm
        bundle={bundle}
        provider={session.provider}
        blocked={controls.busy || (!!persistedRunId && !persisted.data) || (!!viewed && runIsActive(viewed.run))}
        onSubmit={controls.submit}
      />
    </section>
  );
}
