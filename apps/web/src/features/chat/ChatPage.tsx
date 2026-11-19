import { Alert, Button, Dialog, Empty, Form, FormField, Input, List, ListItem, Select, Spinner } from "@bixlab/mtone";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { chatError, createSession, sessionsQuery } from "./queries";

const providerOptions = [
  { value: "local", label: "本地服务" },
  { value: "external", label: "外部服务" },
];

function NewSession({ bundle }: { bundle: QueryClientBundle }) {
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [provider, setProvider] = useState("local");
  const [external, setExternal] = useState("");
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const request = useRef<AbortController | null>(null);
  useEffect(() => () => request.current?.abort(), []);

  async function create() {
    if (request.current) return;
    const selected = provider === "local" ? "local" : external.trim();
    if (!/^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(selected) || (provider === "external" && selected === "local")) {
      setProblem("请输入已配置的外部服务名。");
      return;
    }
    const controller = new AbortController();
    request.current = controller;
    setBusy(true);
    setProblem("");
    try {
      const session = await createSession(bundle, { title: title.trim() || undefined, provider: selected }, controller.signal);
      controller.signal.throwIfAborted();
      setOpen(false);
      setTitle("");
      setProvider("local");
      setExternal("");
      void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.chat.sessions() });
      await navigate({ to: "/chat/$chatSessionId", params: { chatSessionId: session.id }, search: { message: undefined } });
    } catch (error) {
      if (!controller.signal.aborted) setProblem(chatError(error, "无法新建会话，请重试。"));
    } finally {
      request.current = null;
      if (!controller.signal.aborted) setBusy(false);
    }
  }

  return (
    <Dialog
      title="新建会话"
      open={open}
      onOpenChange={(next) => {
        if (busy) return;
        setOpen(next);
        if (next) setProblem("");
      }}
      trigger={<Button variant="solid">新建会话</Button>}
    >
      <Form className="space-y-3" onSubmit={(event) => { event.preventDefault(); void create(); }}>
        <FormField label="会话名称">
          <Input disabled={busy} maxLength={200} value={title} onChange={(event) => setTitle(event.target.value)} />
        </FormField>
        <FormField label="生成服务">
          <Select disabled={busy} options={providerOptions} value={provider} onValueChange={(value) => setProvider(value ?? "local")} />
        </FormField>
        {provider === "external" && <FormField label="外部服务名">
          <Input disabled={busy} value={external} autoComplete="off" onChange={(event) => setExternal(event.target.value)} />
        </FormField>}
        {problem && <Alert tone="error">{problem}</Alert>}
        <div className="flex justify-end gap-2">
          <Button type="button" disabled={busy} onClick={() => setOpen(false)}>返回</Button>
          <Button type="submit" variant="solid" disabled={busy}>创建会话</Button>
        </div>
      </Form>
    </Dialog>
  );
}

export function ChatPage({ bundle, currentSession, children }: {
  bundle: QueryClientBundle;
  currentSession: string | undefined;
  children: ReactNode;
}) {
  const sessions = useInfiniteQuery(sessionsQuery(bundle));
  const rows = sessions.data?.pages.flatMap((page) => page.items) ?? [];
  return (
    <div className="mx-auto grid max-w-7xl gap-6 @min-[42rem]/workspace:grid-cols-[minmax(12rem,1fr)_minmax(0,3fr)]">
      <nav aria-label="会话列表" className={`min-w-0 space-y-3 ${currentSession ? "hidden @min-[42rem]/workspace:block" : ""}`}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          {currentSession ? <h2 className="section-title">会话</h2> : <h1 className="page-title">问答</h1>}
          <NewSession bundle={bundle} />
        </div>
        {sessions.isPending && <Spinner label="正在读取会话列表" />}
        {sessions.isError && <Alert tone="error">
          无法读取会话列表。{" "}<Button onClick={() => void sessions.refetch()}>重试</Button>
        </Alert>}
        {rows.length > 0 && <List>
          {rows.map((session) => (
            <ListItem key={session.id}>
              <Link
                to="/chat/$chatSessionId"
                params={{ chatSessionId: session.id }}
                search={{ message: undefined }}
                aria-current={currentSession === session.id ? "page" : undefined}
                className={`page-link block wrap-anywhere ${currentSession === session.id ? "font-semibold" : ""}`}
              >{session.title}</Link>
            </ListItem>
          ))}
        </List>}
        {sessions.isSuccess && rows.length === 0 && <Empty title="暂无会话" />}
        {sessions.hasNextPage && <Button disabled={sessions.isFetching} onClick={() => void sessions.fetchNextPage()}>更多会话</Button>}
      </nav>
      {currentSession ? (
        <div className="min-w-0 space-y-3">
          <Link className="page-link @min-[42rem]/workspace:hidden" to="/chat">返回会话列表</Link>
          {children}
        </div>
      ) : sessions.isSuccess && rows.length > 0 && (
        <div className="hidden @min-[42rem]/workspace:block"><Empty title="请选择会话" /></div>
      )}
    </div>
  );
}
