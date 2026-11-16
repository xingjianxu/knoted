import { createFileRoute } from "@tanstack/react-router";
import { Empty } from "@bixlab/mtone";
export const Route = createFileRoute("/_authenticated/")({
  component: () => (
    <Empty
      title="欢迎使用 Knoted"
      description="当前可管理 Passkey 与登录会话，笔记及知识库功能将在后续开放。"
    />
  ),
});
