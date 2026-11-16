import {
  createFileRoute,
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { useState } from "react";
import {
  Alert,
  Button,
  Layout,
  NavGroup,
  NavItem,
  Sidebar,
} from "@bixlab/mtone";
import { AuthGate, authError } from "../features/auth/auth";

export const Route = createFileRoute("/_authenticated")({
  beforeLoad: ({ context, location }) => AuthGate(context, location.href),
  component: Shell,
});
function Shell() {
  const bundle = Route.useRouteContext();
  const navigate = useNavigate();
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function logout() {
    setBusy(true);
    setError("");
    try {
      await bundle.api.raw.POST("/auth/logout");
      await bundle.queryClient.cancelQueries();
      bundle.queryClient.clear();
      await navigate({
        to: "/login",
        search: { redirect: "/" },
        replace: true,
      });
    } catch (cause) {
      setError(authError(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Layout fullHeight hasSider>
      <Sidebar
        header={<h1 className="p-4 text-xl font-semibold">Knoted</h1>}
        footer={
          <Button disabled={busy} onClick={() => void logout()}>
            退出登录
          </Button>
        }
      >
        <NavGroup label="工作空间">
          {["概览", "笔记", "项目", "事项", "文件", "搜索", "问答"].map(
            (label) => (
              <NavItem key={label} disabled title="尚未实现">
                {label}（待开放）
              </NavItem>
            ),
          )}
        </NavGroup>
        <NavGroup label="账号">
          <NavItem
            active={pathname === "/settings/security"}
            onClick={() => void navigate({ to: "/settings/security" })}
          >
            凭据与会话
          </NavItem>
        </NavGroup>
      </Sidebar>
      <Layout className="min-w-0 flex-1">
        <Layout.Header>个人工作空间</Layout.Header>
        <Layout.Content className="p-6">
          {error && <Alert tone="error">{error}</Alert>}
          <Outlet />
        </Layout.Content>
      </Layout>
    </Layout>
  );
}
