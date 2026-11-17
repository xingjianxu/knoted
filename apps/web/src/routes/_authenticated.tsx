import {
  createFileRoute,
  Outlet,
  useNavigate,
  useRouterState,
  useRouter,
} from "@tanstack/react-router";
import { useRef, useState } from "react";
import {
  Alert,
  Button,
  Layout,
  NavGroup,
  NavItem,
  Sidebar,
} from "@bixlab/mtone";
import { NotesNavigationBlocked } from "../features/notes/navigation";
import { AuthGate, authError } from "../features/auth/auth";

export const Route = createFileRoute("/_authenticated")({
  beforeLoad: ({ context, location }) => AuthGate(context, location.href),
  component: Shell,
});
function Shell() {
  const bundle = Route.useRouteContext();
  const navigate = useNavigate();
  const router = useRouter();
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const [busy, setBusy] = useState(false);
  const blockedLogout = useRef<(() => void) | null>(null);
  const [error, setError] = useState("");
  async function logout() {
    setBusy(true);
    setError("");
    try {
      if (/^\/notes(?:\/|$)/.test(router.state.location.pathname)) {
        try {
          await new Promise<void>((resolve, reject) => {
            blockedLogout.current = () => reject(new Error("无法安全离开笔记，尚未退出登录。请先处理保存错误。"));
            void navigate({ to: "/" }).then(resolve, reject);
          });
        } finally { blockedLogout.current = null; }
        if (/^\/notes(?:\/|$)/.test(router.state.location.pathname)) {
          throw new Error("无法安全离开笔记，尚未退出登录。请先处理保存错误。");
        }
      }
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
          <NavItem active={/^\/notes(?:\/|$)/.test(pathname)} onClick={() => void navigate({ to: "/notes" })}>笔记</NavItem>
          <NavItem active={/^\/projects(?:\/|$)/.test(pathname)} onClick={() => void navigate({ to: "/projects" })}>项目</NavItem>
          {["概览", "事项", "文件", "搜索", "问答"].map(
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
          <NotesNavigationBlocked.Provider value={() => blockedLogout.current?.()}>
            <Outlet />
          </NotesNavigationBlocked.Provider>
        </Layout.Content>
      </Layout>
    </Layout>
  );
}
