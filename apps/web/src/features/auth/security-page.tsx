import { useReducer, useState } from "react";
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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirmationError, setConfirmationError] = useState("");
  async function confirmAuthOperation(operation: () => Promise<void>) {
    setConfirmationError("");
    try {
      await operation();
    } catch (cause) {
      setConfirmationError(authError(cause));
      throw cause;
    }
  }
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
  async function addPasskey({ name }: z.infer<typeof nameSchema>) {
    setBusy(true);
    setError("");
    try {
      const { data } = await bundle.api.raw.POST("/auth/passkeys/options");
      if (!data) throw new Error("无法开始注册，请重试。");
      const credential = await runWebAuthn(data.options, "create");
      await bundle.api.raw.POST("/auth/passkeys/finish", {
        body: { ceremony_id: data.ceremony_id, credential, name },
      });
      reset();
      passkeyDispatch({
        type: "filter",
        filterKey: String(Number(passkeyCursor.filterKey) + 1),
      });
      await bundle.queryClient.invalidateQueries({
        queryKey: queryKeys.auth.passkeys(),
      });
    } catch (cause) {
      setError(authError(cause));
    } finally {
      setBusy(false);
    }
  }
  async function deletePasskey(id: string) {
    setError("");
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
    <div className="grid max-w-3xl gap-8">
      <h2 className="text-2xl font-semibold">凭据与会话</h2>
      <p>
        新增或删除 Passkey 需要最近五分钟内的登录验证。
        <Link to="/login" search={{ redirect: "/settings/security" }}>
          重新验证
        </Link>
      </p>
      {error && <Alert tone="error">{error}</Alert>}
      <section className="space-y-4" aria-labelledby="passkeys-title">
        <h3 id="passkeys-title" className="text-xl">
          Passkey 凭据
        </h3>
        <form
          className="flex items-end gap-3"
          onSubmit={(event) => void handleSubmit(addPasskey)(event)}
        >
          <FormField label="新凭据名称" error={errors.name?.message}>
            <Input {...register("name")} disabled={busy} />
          </FormField>
          <Button type="submit" disabled={busy}>
            添加 Passkey
          </Button>
        </form>
        {passkeys.isPending && <Spinner label="正在加载凭据" />}
        {passkeys.isError && (
          <>
            <Alert tone="error">{authError(passkeys.error)}</Alert>
            <Button onClick={() => void passkeys.refetch()}>重试凭据</Button>
          </>
        )}
        {passkeys.data?.items.length === 0 && <Empty title="没有凭据" />}
        {passkeys.data && (
          <List>
            {passkeys.data.items.map((passkey) => (
              <ListItem key={passkey.id}>
                <div className="flex w-full items-center justify-between gap-4">
                  <div>
                    <strong>{passkey.name}</strong>
                    <p>
                      创建于 {new Date(passkey.created_at).toLocaleString()}
                    </p>
                  </div>
                  <ConfirmDialog
                    title={`删除凭据 ${passkey.name}`}
                    description="删除后将无法使用此凭据登录；最后一个凭据不能删除。"
                    confirmText="确认删除"
                    onOpenChange={() => setConfirmationError("")}
                    onConfirm={() =>
                      confirmAuthOperation(() => deletePasskey(passkey.id))
                    }
                    trigger={
                      <Button
                        disabled={
                          busy ||
                          (passkeyCursor.cursor === null &&
                            passkeys.data.next_cursor === null &&
                            passkeys.data.items.length === 1)
                        }
                      >
                        删除 {passkey.name}
                      </Button>
                    }
                  >
                    {confirmationError && (
                      <p role="status">{confirmationError}</p>
                    )}
                  </ConfirmDialog>
                </div>
              </ListItem>
            ))}
          </List>
        )}
        <div className="flex gap-3">
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
        </div>
      </section>
      <section className="space-y-4" aria-labelledby="sessions-title">
        <h3 id="sessions-title" className="text-xl">
          登录会话
        </h3>
        {sessions.isPending && <Spinner label="正在加载会话" />}
        {sessions.isError && (
          <>
            <Alert tone="error">{authError(sessions.error)}</Alert>
            <Button onClick={() => void sessions.refetch()}>重试会话</Button>
          </>
        )}
        {sessions.data?.items.length === 0 && <Empty title="没有会话" />}
        {sessions.data && (
          <List>
            {sessions.data.items.map((session) => (
              <ListItem key={session.id}>
                <div className="flex w-full items-center justify-between gap-4">
                  <div>
                    <strong>{session.current ? "当前会话" : "其他会话"}</strong>
                    <p>
                      最近使用 {new Date(session.last_seen_at).toLocaleString()}
                    </p>
                  </div>
                  <ConfirmDialog
                    title={session.current ? "退出当前会话" : "撤销其他会话"}
                    description="撤销后需要重新使用 Passkey 登录。"
                    confirmText="确认撤销"
                    onOpenChange={() => setConfirmationError("")}
                    onConfirm={() =>
                      confirmAuthOperation(() =>
                        revokeSession(session.id, session.current),
                      )
                    }
                    trigger={
                      <Button>
                        撤销{session.current ? "当前" : "其他"}会话
                      </Button>
                    }
                  >
                    {confirmationError && (
                      <p role="status">{confirmationError}</p>
                    )}
                  </ConfirmDialog>
                </div>
              </ListItem>
            ))}
          </List>
        )}
        <div className="flex gap-3">
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
        </div>
      </section>
    </div>
  );
}
