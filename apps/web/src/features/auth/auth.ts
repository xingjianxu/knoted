import { queryOptions } from "@tanstack/react-query";
import { ApiRequestError } from "@knoted/api-client";
import { redirect } from "@tanstack/react-router";
import { queryKeys, type QueryClientBundle } from "../../lib/query";

export function sessionQuery(bundle: QueryClientBundle) {
  return queryOptions({
    queryKey: queryKeys.auth.session(),
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/auth/session", { signal });
      if (!data) throw new Error("无法读取会话，请重试。");
      return data;
    },
    retry: false,
    staleTime: 0,
  });
}
/** AuthGate revalidates server authentication before protected routes render. */
export async function AuthGate(bundle: QueryClientBundle, href: string) {
  try {
    await bundle.queryClient.fetchQuery(sessionQuery(bundle));
  } catch (error) {
    if (error instanceof ApiRequestError && error.status === 401) {
      bundle.queryClient.clear();
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
    if (error.status === 403)
      return "此操作需要重新登录验证，请重新使用 Passkey 登录。";
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
