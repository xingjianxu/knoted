import { useEffect, useRef, useState } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Alert, Button, Card, Divider, Form, Input, Paragraph, Text, Title } from "@bixlab/mtone";
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
  useEffect(() => () => {
    const pending = ceremony.current;
    ceremony.current = null;
    pending?.abort();
  }, []);
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
    if (ceremony.current) return;
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
      signal.throwIfAborted();
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
      signal.throwIfAborted();
      await bundle.queryClient.cancelQueries();
      signal.throwIfAborted();
      bundle.queryClient.clear();
      bundle.queryClient.setQueryData(queryKeys.auth.session(), finished.data);
      resetField("token");
      await navigate({ href: safeReturnTo(returnTo), replace: true });
    } catch (cause) {
      if (!signal.aborted) setError(authError(cause));
    } finally {
      if (ceremony.current === controller) {
        ceremony.current = null;
        setBusy(false);
      }
    }
  }
  const title =
    mode === "login"
      ? "登录"
      : mode === "bootstrap"
        ? "首次注册"
        : "恢复账号";
  return (
    <main aria-labelledby="auth-title" className="grid min-h-dvh place-items-center p-3 sm:p-6">
      <Card className="w-full min-w-0 max-w-sm">
        <div className="space-y-6">
          <div className="space-y-2">
            <Text strong className="block text-accent">Knoted</Text>
            <Title level={1} id="auth-title" className="page-title">{title}</Title>
          </div>
          {mode === "login" ? (
            <div className="space-y-3">
              <Button className="w-full" variant="solid" size="lg" loading={busy} focusableWhenDisabled
                onClick={() => void authenticate()}>
                使用 Passkey 登录
              </Button>
              {error && <Alert tone="error">{error}</Alert>}
            </div>
          ) : (
            <Form aria-labelledby="auth-title" className="gap-4" onSubmit={(event) => void handleSubmit(authenticate)(event)}>
              {mode === "bootstrap" ? (
                <Paragraph muted>使用管理员在本机签发的一次性注册令牌。</Paragraph>
              ) : (
                <Alert tone="warning">恢复成功后将撤销所有旧凭据与会话。</Alert>
              )}
              <Form.Item label="一次性令牌" error={errors.token?.message}>
                <Input
                  id="token"
                  type="password"
                  autoComplete="off"
                  {...register("token")}
                  disabled={busy}
                />
              </Form.Item>
              <Form.Item label="凭据名称" error={errors.name?.message}>
                <Input
                  id="name"
                  autoComplete="off"
                  {...register("name")}
                  disabled={busy}
                />
              </Form.Item>
              {error && <Alert tone="error">{error}</Alert>}
              <Button type="submit" className="w-full" variant="solid" size="lg" loading={busy} focusableWhenDisabled>
                {mode === "bootstrap" ? "注册 Passkey" : "替换凭据并恢复"}
              </Button>
            </Form>
          )}
          <Divider />
          <nav className="flex flex-wrap justify-center gap-x-6 gap-y-2" aria-label="认证方式">
            {mode === "login" ? (
              <>
                <Link className="page-link" to="/bootstrap">首次注册</Link>
                <Link className="page-link" to="/recovery">恢复账号</Link>
              </>
            ) : (
              <Link className="page-link" to="/login" search={{ redirect: safeReturnTo(returnTo) }}>返回登录</Link>
            )}
          </nav>
        </div>
      </Card>
    </main>
  );
}
