import { createFileRoute } from "@tanstack/react-router";
import { Empty } from "@bixlab/mtone";
export const Route = createFileRoute("/_authenticated/")({
  component: () => (
    <Empty
      title="欢迎使用 Knoted"
      description="当前可编辑笔记、查看历史，并管理 Passkey 与登录会话。其他知识库功能将陆续开放。"
    />
  ),
});
