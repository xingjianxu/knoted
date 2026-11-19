import {
  createRootRouteWithContext,
  Link,
  Outlet,
  useRouter,
} from "@tanstack/react-router";
import { Alert, Button } from "@bixlab/mtone";
import { useState } from "react";
import type { QueryClientBundle } from "../lib/query";

function RouteError() {
  const router = useRouter();
  const [retrying, setRetrying] = useState(false);
  async function retry() {
    if (retrying) return;
    setRetrying(true);
    try {
      await router.invalidate();
    } finally {
      setRetrying(false);
    }
  }
  return (
    <main aria-labelledby="route-error-title" className="reading-measure mx-auto space-y-4 px-3 py-6 sm:px-6">
      <h1 id="route-error-title" className="page-title">无法加载页面</h1>
      <Alert tone="error">请检查连接后重试。</Alert>
      <div className="flex flex-wrap items-center gap-3">
        <Button autoFocus focusableWhenDisabled loading={retrying} onClick={() => void retry()}>重试</Button>
        <span role="status" className="text-muted">{retrying ? "正在重试…" : ""}</span>
      </div>
    </main>
  );
}
export const Route = createRootRouteWithContext<QueryClientBundle>()({
  component: Outlet,
  errorComponent: RouteError,
  notFoundComponent: () => (
    <main aria-labelledby="not-found-title" className="reading-measure mx-auto space-y-4 px-3 py-6 sm:px-6">
      <h1 id="not-found-title" className="page-title">页面不存在</h1>
      <Link to="/" className="page-link">返回首页</Link>
    </main>
  ),
});
