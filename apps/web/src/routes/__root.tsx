import {
  createRootRouteWithContext,
  Link,
  Outlet,
  useRouter,
} from "@tanstack/react-router";
import { Alert, Button, Result, Text, Title } from "@bixlab/mtone";
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
    <main aria-labelledby="route-error-title" className="reading-measure mx-auto px-3 py-6 sm:px-6">
      <Result
        status="error"
        title={<Title level={1} id="route-error-title" className="page-title">无法加载页面</Title>}
        extra={
          <div className="flex flex-wrap items-center justify-center gap-3">
            <Button autoFocus focusableWhenDisabled loading={retrying} onClick={() => void retry()}>重试</Button>
            <Text role="status" muted>{retrying ? "正在重试…" : ""}</Text>
          </div>
        }
      >
        <Alert tone="error">请检查连接后重试。</Alert>
      </Result>
    </main>
  );
}
export const Route = createRootRouteWithContext<QueryClientBundle>()({
  component: Outlet,
  errorComponent: RouteError,
  notFoundComponent: () => (
    <main aria-labelledby="not-found-title" className="reading-measure mx-auto px-3 py-6 sm:px-6">
      <Result
        status="404"
        title={<Title level={1} id="not-found-title" className="page-title">页面不存在</Title>}
        extra={<Link to="/" className="page-link">返回首页</Link>}
      />
    </main>
  ),
});
