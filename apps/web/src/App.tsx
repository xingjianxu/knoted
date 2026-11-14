import { cn } from "@knoted/ui";

export default function App() {
  return (
    <main
      className={cn(
        "flex min-h-screen items-center justify-center bg-canvas p-6",
        "text-ink",
      )}
    >
      <section className="w-full max-w-xl rounded-panel border border-line bg-surface p-8">
        <h1 className="font-sans text-3xl font-semibold tracking-tight">
          Knoted
        </h1>
        <p className="mt-3 text-muted">
          应用外壳已就绪，笔记与知识库功能将在后续任务中接入。
        </p>
      </section>
    </main>
  );
}
