import {
  createRootRouteWithContext,
  Outlet,
  useRouter,
} from "@tanstack/react-router";
import { Alert, Button } from "@bixlab/mtone";
import type { QueryClientBundle } from "../lib/query";

function RouteError() {
  const router = useRouter();
  return (
    <main className="p-6">
      <Alert tone="error">无法加载页面，请检查连接后重试。</Alert>
      <Button onClick={() => void router.invalidate()}>重试</Button>
    </main>
  );
}
export const Route = createRootRouteWithContext<QueryClientBundle>()({
  component: Outlet,
  errorComponent: RouteError,
  notFoundComponent: () => (
    <main className="p-6">
      <h1>页面不存在</h1>
      <a href="/">返回首页</a>
    </main>
  ),
});
