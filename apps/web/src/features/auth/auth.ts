import { ApiRequestError } from "@knoted/api-client";
import { redirect } from "@tanstack/react-router";
import { queryKeys, type QueryClientBundle } from "../../lib/query";

/** AuthGate revalidates server authentication before protected routes render. */
export async function AuthGate(bundle: QueryClientBundle, href: string, signal: AbortSignal) {
  try {
    // The router owns this request. A 401 evicts cached queries as soon as
    // headers arrive, while the route still needs to receive the parsed error.
    const { data } = await bundle.api.raw.GET("/auth/session", { signal });
    signal.throwIfAborted();
    if (!data) throw new Error("无法读取会话，请重试。");
    bundle.queryClient.setQueryData(queryKeys.auth.session(), data);
  } catch (error) {
    signal.throwIfAborted();
    if (error instanceof ApiRequestError && error.status === 401) {
      bundle.queryClient.getMutationCache().clear();
      throw redirect({
        to: "/login",
        search: { redirect: href },
        replace: true,
      });
    }
    throw error;
  }
}
export function authError(error: unknown): string {
  if (
    error instanceof DOMException &&
    ["NotAllowedError", "AbortError"].includes(error.name)
  )
    return "认证已取消或超时，请重试。";
  if (error instanceof ApiRequestError) {
    if (error.code === "recent_verification_required")
      return "此操作需要重新登录验证，请重新使用 Passkey 登录。";
    if (error.code === "csrf_failed")
      return "无法验证此页面的请求，请刷新页面后重试。";
    if (error.status === 403) return "没有权限执行此操作。";
    if (error.status === 409)
      return "操作冲突或凭据不可删除（不能删除最后一个凭据），请刷新后重试。";
    if (error.status === 401) return "会话已失效，请重新登录。";
    if (error.status === 422) return "认证信息无效，请重新开始。";
    return "服务暂时不可用，请重试。";
  }
  if (error instanceof Error && !(error instanceof TypeError))
    return error.message;
  return "连接失败，请检查网络后重试。";
}
