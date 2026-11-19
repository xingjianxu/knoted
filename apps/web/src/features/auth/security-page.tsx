import { useEffect, useReducer, useRef, useState, type ReactElement } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useRouteContext } from "@tanstack/react-router";
import {
  Alert,
  Button,
  ConfirmDialog,
  Empty,
  FormField,
  Input,
  List,
  ListItem,
  Spinner,
} from "@bixlab/mtone";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { ApiRequestError } from "@knoted/api-client";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { queryKeys } from "../../lib/query";
import { authError } from "./auth";
import { runWebAuthn } from "./webauthn";

const nameSchema = z.object({
  name: z.string().trim().min(1, "请输入凭据名称").max(100, "名称最多 100 字"),
});
export function SecurityPage() {
  const bundle = useRouteContext({ from: "__root__" });
  const navigate = useNavigate();
  const [passkeyCursor, passkeyDispatch] = useReducer(
    cursorReducer,
    undefined,
    () => createCursorState(),
  );
  const [sessionCursor, sessionDispatch] = useReducer(
    cursorReducer,
    undefined,
    () => createCursorState(),
  );
  const addTrigger = useRef<HTMLButtonElement>(null);
  const ceremony = useRef<AbortController | null>(null);
  const [showAddForm, setShowAddForm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => () => {
    const pending = ceremony.current;
    ceremony.current = null;
    pending?.abort();
  }, []);
  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<z.infer<typeof nameSchema>>({
    resolver: zodResolver(nameSchema),
    defaultValues: { name: "新 Passkey" },
  });
  const passkeys = useQuery({
    queryKey: [...queryKeys.auth.passkeys(), passkeyCursor.cursor],
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/auth/passkeys", {
        signal,
        params: {
          query: { cursor: passkeyCursor.cursor ?? undefined, limit: 50 },
        },
      });
      if (!data) throw new Error("无法读取凭据，请重试。");
      return data;
    },
  });
  const sessions = useQuery({
    queryKey: [...queryKeys.auth.sessions(), sessionCursor.cursor],
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/auth/sessions", {
        signal,
        params: {
          query: { cursor: sessionCursor.cursor ?? undefined, limit: 50 },
        },
      });
      if (!data) throw new Error("无法读取会话，请重试。");
      return data;
    },
  });
  function closeAddForm() {
    reset();
    setError("");
    setShowAddForm(false);
    addTrigger.current?.focus({ preventScroll: true });
  }
  async function addPasskey({ name }: z.infer<typeof nameSchema>) {
    if (ceremony.current) return;
    const controller = new AbortController();
    ceremony.current = controller;
    const { signal } = controller;
    setBusy(true);
    setError("");
    try {
      const { data } = await bundle.api.raw.POST("/auth/passkeys/options", { signal });
      signal.throwIfAborted();
      if (!data) throw new Error("无法开始注册，请重试。");
      const credential = await runWebAuthn(data.options, "create", signal);
      await bundle.api.raw.POST("/auth/passkeys/finish", {
        body: { ceremony_id: data.ceremony_id, credential, name }, signal,
      });
      signal.throwIfAborted();
      passkeyDispatch({
        type: "filter",
        filterKey: String(Number(passkeyCursor.filterKey) + 1),
      });
      await bundle.queryClient.invalidateQueries({
        queryKey: queryKeys.auth.passkeys(),
      });
      if (!signal.aborted) closeAddForm();
    } catch (cause) {
      if (!signal.aborted) setError(authError(cause));
    } finally {
      if (ceremony.current === controller) {
        ceremony.current = null;
        setBusy(false);
      }
    }
  }
  async function deletePasskey(id: string) {
    await bundle.api.raw.DELETE("/auth/passkeys/{passkeyId}", {
      params: { path: { passkeyId: id } },
    });
    passkeyDispatch({
      type: "filter",
      filterKey: String(Number(passkeyCursor.filterKey) + 1),
    });
    await bundle.queryClient.invalidateQueries({
      queryKey: queryKeys.auth.passkeys(),
    });
  }
  async function revokeSession(id: string, current: boolean) {
    await bundle.api.raw.DELETE("/auth/sessions/{sessionId}", {
      params: { path: { sessionId: id } },
    });
    if (current) {
      await bundle.queryClient.cancelQueries();
      bundle.queryClient.clear();
      await navigate({
        to: "/login",
        search: { redirect: "/settings/security" },
        replace: true,
      });
    } else {
      sessionDispatch({
        type: "filter",
        filterKey: String(Number(sessionCursor.filterKey) + 1),
      });
      await bundle.queryClient.invalidateQueries({
        queryKey: queryKeys.auth.sessions(),
      });
    }
  }
  return (
    <div className="mx-auto grid w-full min-w-0 max-w-[45rem] gap-6">
      <h1 className="page-title">凭据与会话</h1>
      <section className="min-w-0 space-y-4 rounded-panel border border-line bg-surface p-3 sm:p-4" aria-labelledby="passkeys-title">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 id="passkeys-title" className="section-title">Passkey 凭据</h2>
          <Button ref={addTrigger} variant={showAddForm ? "outline" : "solid"} disabled={busy} focusableWhenDisabled
            aria-expanded={showAddForm} aria-controls={showAddForm ? "add-passkey" : undefined}
            onClick={() => showAddForm ? closeAddForm() : setShowAddForm(true)}>
            添加 Passkey
          </Button>
        </div>
        {showAddForm && (
          <form id="add-passkey" aria-label="添加 Passkey" className="space-y-3 rounded-panel border border-line bg-canvas p-3"
            onSubmit={(event) => void handleSubmit(addPasskey)(event)}>
            <p className="text-muted">新增或删除 Passkey 需要最近五分钟内的登录验证。 <ReverifyLink /></p>
            <FormField label="新凭据名称" error={errors.name?.message}>
              <Input autoFocus {...register("name")} disabled={busy} />
            </FormField>
            {error && <Alert tone="error">{error}</Alert>}
            <div className="flex flex-wrap gap-2">
              <Button type="submit" variant="solid" loading={busy} focusableWhenDisabled>注册 Passkey</Button>
              <Button disabled={busy} onClick={closeAddForm}>取消</Button>
            </div>
          </form>
        )}
        {passkeys.isPending && <Spinner label="正在加载凭据" />}
        {passkeys.isError && (
          <>
            <Alert tone="error">{authError(passkeys.error)}</Alert>
            <Button loading={passkeys.isFetching} focusableWhenDisabled onClick={() => void passkeys.refetch()}>重试凭据</Button>
          </>
        )}
        {passkeys.data?.items.length === 0 && <Empty title="没有凭据" />}
        {!!passkeys.data?.items.length && (
          <List aria-labelledby="passkeys-title">
            {passkeys.data.items.map((passkey) => (
              <ListItem key={passkey.id}>
                <div className="flex w-full flex-wrap items-center gap-3">
                  <div className="min-w-0 flex-1 basis-48 wrap-anywhere">
                    <strong>{passkey.name}</strong>
                    <p className="text-sm text-muted">创建于 <AccountTime value={passkey.created_at} /></p>
                  </div>
                  <SecurityConfirmation
                    title={`删除凭据 ${passkey.name}`}
                    description="删除后将无法使用此凭据登录；最后一个凭据不能删除。"
                    confirmText="确认删除"
                    onConfirm={() => deletePasskey(passkey.id)}
                    trigger={
                      <Button aria-label={`删除 ${passkey.name}`} variant="ghost"
                        disabled={busy || (passkeyCursor.cursor === null &&
                          passkeys.data.next_cursor === null && passkeys.data.items.length === 1)}>
                        删除
                      </Button>
                    }
                  />
                </div>
              </ListItem>
            ))}
          </List>
        )}
        <nav aria-label="凭据分页" className="flex flex-wrap gap-2">
          <Button
            disabled={
              passkeys.isFetching || passkeyCursor.previous.length === 0
            }
            onClick={() => passkeyDispatch({ type: "previous" })}
          >
            上一页凭据
          </Button>
          <Button
            disabled={passkeys.isFetching || !passkeys.data?.next_cursor}
            onClick={() =>
              passkeyDispatch({
                type: "next",
                nextCursor: passkeys.data?.next_cursor ?? null,
              })
            }
          >
            下一页凭据
          </Button>
        </nav>
      </section>
      <section className="min-w-0 space-y-4 rounded-panel border border-line bg-surface p-3 sm:p-4" aria-labelledby="sessions-title">
        <h2 id="sessions-title" className="section-title">登录会话</h2>
        {sessions.isPending && <Spinner label="正在加载会话" />}
        {sessions.isError && (
          <>
            <Alert tone="error">{authError(sessions.error)}</Alert>
            <Button loading={sessions.isFetching} focusableWhenDisabled onClick={() => void sessions.refetch()}>重试会话</Button>
          </>
        )}
        {sessions.data?.items.length === 0 && <Empty title="没有会话" />}
        {!!sessions.data?.items.length && (
          <List aria-labelledby="sessions-title">
            {sessions.data.items.map((session) => (
              <ListItem key={session.id}>
                <div className="flex w-full flex-wrap items-center gap-3">
                  <div className="min-w-0 flex-1 basis-48">
                    <strong className={session.current ? "text-accent" : undefined}>
                      {session.current ? "当前会话" : "其他会话"}
                    </strong>
                    <p className="text-sm text-muted">最近使用 <AccountTime value={session.last_seen_at} /></p>
                  </div>
                  <SecurityConfirmation
                    title={session.current ? "退出当前会话" : "撤销其他会话"}
                    description="撤销后需要重新使用 Passkey 登录。"
                    confirmText="确认撤销"
                    onConfirm={() => revokeSession(session.id, session.current)}
                    trigger={<Button variant="ghost">撤销{session.current ? "当前" : "其他"}会话</Button>}
                  />
                </div>
              </ListItem>
            ))}
          </List>
        )}
        <nav aria-label="会话分页" className="flex flex-wrap gap-2">
          <Button
            disabled={
              sessions.isFetching || sessionCursor.previous.length === 0
            }
            onClick={() => sessionDispatch({ type: "previous" })}
          >
            上一页会话
          </Button>
          <Button
            disabled={sessions.isFetching || !sessions.data?.next_cursor}
            onClick={() =>
              sessionDispatch({
                type: "next",
                nextCursor: sessions.data?.next_cursor ?? null,
              })
            }
          >
            下一页会话
          </Button>
        </nav>
      </section>
    </div>
  );
}

function ReverifyLink() {
  return <Link className="page-link" to="/login" search={{ redirect: "/settings/security" }}>重新验证</Link>;
}

function AccountTime({ value }: { value: string }) {
  return <time dateTime={value}>{new Date(value).toLocaleString()}</time>;
}

function SecurityConfirmation({ title, description, confirmText, trigger, onConfirm }: {
  title: string;
  description: string;
  confirmText: string;
  trigger: ReactElement;
  onConfirm: () => Promise<void>;
}) {
  const [error, setError] = useState<{ message: string; reverify: boolean } | null>(null);
  return (
    <ConfirmDialog title={<span className="wrap-anywhere">{title}</span>} description={description}
      confirmText={confirmText} danger trigger={trigger} onOpenChange={() => setError(null)}
      onConfirm={async () => {
        setError(null);
        try {
          await onConfirm();
        } catch (cause) {
          setError({
            message: authError(cause),
            reverify: cause instanceof ApiRequestError && cause.code === "recent_verification_required",
          });
          throw cause;
        }
      }}>
      {error && <p role="status" className="wrap-anywhere">{error.message} {error.reverify && <ReverifyLink />}</p>}
    </ConfirmDialog>
  );
}
