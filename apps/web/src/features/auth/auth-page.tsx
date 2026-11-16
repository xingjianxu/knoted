import { useEffect, useRef, useState } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Alert, Button, FormField, Input } from "@bixlab/mtone";
import { Link, useNavigate, useRouteContext } from "@tanstack/react-router";
import { queryKeys } from "../../lib/query";
import { safeReturnTo } from "../../lib/router";
import { authError } from "./auth";
import { runWebAuthn } from "./webauthn";

const schema = z.object({
  token: z.string().min(1, "请输入一次性令牌"),
  name: z.string().trim().min(1, "请输入凭据名称").max(100, "名称最多 100 字"),
});
type Values = z.infer<typeof schema>;
export function AuthPage({
  mode,
  returnTo = "/",
}: {
  mode: "login" | "bootstrap" | "recovery";
  returnTo?: string;
}) {
  const bundle = useRouteContext({ from: "__root__" });
  const navigate = useNavigate();
  const ceremony = useRef<AbortController | null>(null);
  useEffect(() => () => ceremony.current?.abort(), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const {
    register,
    handleSubmit,
    resetField,
    formState: { errors },
  } = useForm<Values>({
    resolver: zodResolver(schema),
    defaultValues: { token: "", name: "我的 Passkey" },
  });
  async function authenticate(values?: Values) {
    ceremony.current?.abort();
    const controller = new AbortController();
    ceremony.current = controller;
    const signal = controller.signal;
    setBusy(true);
    setError("");
    try {
      const result =
        mode === "login"
          ? await bundle.api.raw.POST("/auth/login/options", { signal })
          : mode === "bootstrap"
            ? await bundle.api.raw.POST("/auth/bootstrap/options", {
                body: { bootstrap_token: values!.token },
                signal,
              })
            : await bundle.api.raw.POST("/auth/recovery/options", {
                body: { recovery_token: values!.token },
                signal,
              });
      if (!result.data) throw new Error("无法开始认证，请重试。");
      const credential = await runWebAuthn(
        result.data.options,
        mode === "login" ? "get" : "create",
        signal,
      );
      const body = { ceremony_id: result.data.ceremony_id, credential };
      const finished =
        mode === "login"
          ? await bundle.api.raw.POST("/auth/login/finish", { body, signal })
          : mode === "bootstrap"
            ? await bundle.api.raw.POST("/auth/bootstrap/finish", {
                body: { ...body, name: values!.name },
                signal,
              })
            : await bundle.api.raw.POST("/auth/recovery/finish", {
                body: {
                  ...body,
                  name: values!.name,
                  recovery_token: values!.token,
                },
                signal,
              });
      if (!finished.data) throw new Error("认证未完成，请重试。");
      await bundle.queryClient.cancelQueries();
      bundle.queryClient.clear();
      bundle.queryClient.setQueryData(queryKeys.auth.session(), finished.data);
      resetField("token");
      await navigate({ href: safeReturnTo(returnTo), replace: true });
    } catch (cause) {
      if (!signal.aborted) setError(authError(cause));
    } finally {
      setBusy(false);
    }
  }
  const title =
    mode === "login"
      ? "登录 Knoted"
      : mode === "bootstrap"
        ? "首次注册"
        : "恢复账号";
  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <section className="w-full max-w-md space-y-5">
        <h1 className="text-2xl font-semibold">{title}</h1>
        <p>使用 Passkey 安全访问个人工作空间。</p>
        {error && <Alert tone="error">{error}</Alert>}
        {mode === "login" ? (
          <Button disabled={busy} onClick={() => void authenticate()}>
            使用 Passkey 登录
          </Button>
        ) : (
          <form
            className="space-y-4"
            onSubmit={(event) => void handleSubmit(authenticate)(event)}
          >
            <p>
              {mode === "bootstrap"
                ? "使用管理员在本机签发的一次性注册令牌。"
                : "恢复成功后将撤销所有旧凭据与会话。"}
            </p>
            <FormField label="一次性令牌" error={errors.token?.message}>
              <Input
                id="token"
                type="password"
                autoComplete="off"
                {...register("token")}
                disabled={busy}
              />
            </FormField>
            <FormField label="凭据名称" error={errors.name?.message}>
              <Input
                id="name"
                autoComplete="off"
                {...register("name")}
                disabled={busy}
              />
            </FormField>
            <Button type="submit" disabled={busy}>
              {mode === "bootstrap" ? "注册 Passkey" : "替换凭据并恢复"}
            </Button>
          </form>
        )}
        <nav className="flex gap-4" aria-label="认证方式">
          <Link to="/login" search={{ redirect: safeReturnTo(returnTo) }}>
            登录
          </Link>
          <Link to="/bootstrap">首次注册</Link>
          <Link to="/recovery">恢复账号</Link>
        </nav>
      </section>
    </main>
  );
}
