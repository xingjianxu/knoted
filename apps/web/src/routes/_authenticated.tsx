import {
  createFileRoute,
  Outlet,
  useNavigate,
  useRouterState,
  useRouter,
} from "@tanstack/react-router";
import { useRef, useState } from "react";
import { AdminLayout, Alert, Button, NavGroup, NavItem } from "@bixlab/mtone";
import {
  BookOpen,
  FolderKanban,
  House,
  LibraryBig,
  ListTodo,
  MessageSquare,
  Search,
  ShieldCheck,
  Trash2,
} from "lucide-react";
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
    <AdminLayout fullHeight>
      <AdminLayout.Sider
        logo={<strong>Knoted</strong>}
        collapsedLogo={<strong><span aria-hidden="true">K</span><span className="sr-only">Knoted</span></strong>}
        collapsible
        breakpoint={768}
      >
        <nav aria-label="主导航">
          <NavGroup label="工作空间">
            <NavItem icon={<House size={16} aria-hidden="true" />} active={pathname === "/"}
              onClick={() => void navigate({ to: "/" })}>概览</NavItem>
            <NavItem icon={<BookOpen size={16} aria-hidden="true" />} active={/^\/notes(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/notes" })}>笔记</NavItem>
            <NavItem icon={<FolderKanban size={16} aria-hidden="true" />} active={/^\/projects(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/projects" })}>项目</NavItem>
            <NavItem icon={<ListTodo size={16} aria-hidden="true" />} active={/^\/tasks(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/tasks" })}>事项</NavItem>
            <NavItem icon={<Trash2 size={16} aria-hidden="true" />} active={pathname === "/trash"}
              onClick={() => void navigate({ to: "/trash" })}>回收站</NavItem>
            <NavItem icon={<LibraryBig size={16} aria-hidden="true" />} active={/^\/files(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/files" })}>文件</NavItem>
            <NavItem icon={<Search size={16} aria-hidden="true" />} active={pathname === "/search"}
              onClick={() => void navigate({ to: "/search", search: { q: "" } })}>搜索</NavItem>
            <NavItem icon={<MessageSquare size={16} aria-hidden="true" />} active={/^\/chat(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/chat" })}>问答</NavItem>
          </NavGroup>
          <NavGroup label="账号">
            <NavItem icon={<ShieldCheck size={16} aria-hidden="true" />} active={pathname === "/settings/security"}
              onClick={() => void navigate({ to: "/settings/security" })}>凭据与会话</NavItem>
          </NavGroup>
        </nav>
      </AdminLayout.Sider>
      <AdminLayout className="min-w-0 flex-1">
        <AdminLayout.Header extra={
          <Button disabled={busy} onClick={() => void logout()}>退出登录</Button>
        } />
        <AdminLayout.Content data-workspace-content className="p-3 sm:p-6">
          {error && <Alert tone="error">{error}</Alert>}
          <NotesNavigationBlocked.Provider value={() => blockedLogout.current?.()}>
            <Outlet />
          </NotesNavigationBlocked.Provider>
        </AdminLayout.Content>
      </AdminLayout>
    </AdminLayout>
  );
}
