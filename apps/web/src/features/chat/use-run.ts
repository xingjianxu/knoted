import { ApiRequestError } from "@knoted/api-client";
import { useEffect, useRef, useState } from "react";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { changeRun, chatError, postQuestion, readRun, type ChatRequest, type ChatRun, type ChatSessionDetail, type Citation } from "./queries";
import { readChatEvents, type ChatEvent } from "./stream";

export type ViewedRun = {
  run: ChatRun;
  body: string;
  citations: Citation[];
  disconnected: boolean;
  stopping: boolean;
};

export function runIsActive(run: ChatRun) {
  return run.state === "queued" || run.state === "started";
}

export function viewRun(run: ChatRun): ViewedRun {
  return { run, body: "", citations: [], disconnected: false, stopping: false };
}

function applyEvent(view: ViewedRun, event: ChatEvent): ViewedRun {
  const run = view.run;
  switch (event.type) {
    case "queued":
      return view;
    case "started":
      return { ...view, run: { ...run, state: "started" } };
    case "delta":
      return { ...view, run: { ...run, state: "started", partial: true }, body: view.body + (event.delta ?? "") };
    case "citation":
      if (!event.citation || view.citations.some(({ id }) => id === event.citation?.id)) return view;
      return { ...view, citations: [...view.citations, event.citation] };
    case "completed":
      return { ...view, run: { ...run, state: "completed", partial: false }, stopping: false };
    case "failed":
    case "cancelled":
      return {
        ...view,
        run: { ...run, state: event.type, partial: true, error: event.error?.code ?? null },
        stopping: false,
      };
  }
}

export function useChatRun(bundle: QueryClientBundle, sessionId: string) {
  const [current, setCurrent] = useState<ViewedRun>();
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const streaming = useRef<{ id: string; controller: AbortController } | null>(null);
  const command = useRef<AbortController | null>(null);
  const pendingIntent = useRef<{ body: string; key: string } | null>(null);

  useEffect(() => () => {
    streaming.current?.controller.abort();
    command.current?.abort();
  }, []);

  function refreshHistory() {
    void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.chat.session(sessionId) });
    void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.chat.sessions() });
  }

  function observe(run: ChatRun) {
    streaming.current?.controller.abort();
    const controller = new AbortController();
    streaming.current = { id: run.id, controller };
    setCurrent(viewRun(run));

    function update(change: (view: ViewedRun) => ViewedRun) {
      if (streaming.current?.controller !== controller || controller.signal.aborted) return;
      setCurrent((view) => view?.run.id === run.id ? change(view) : view);
    }

    async function listen() {
      try {
        const terminal = await readChatEvents(bundle, run.id, controller.signal, (event) => update((view) => applyEvent(view, event)));
        if (controller.signal.aborted) return;
        if (terminal) {
          refreshHistory();
          return;
        }
      } catch {
        if (controller.signal.aborted) return;
      }
      update((view) => ({ ...view, disconnected: true }));
      try {
        const status = await readRun(bundle, sessionId, run.id, controller.signal);
        update((view) => ({ ...view, run: status, stopping: view.stopping && runIsActive(status) }));
        if (!controller.signal.aborted && !runIsActive(status)) refreshHistory();
      } catch {
        // The saved partial answer remains visible until an explicit status check.
      }
    }
    void listen();
  }

  async function perform(action: (signal: AbortSignal) => Promise<void>, fallback: string) {
    if (command.current) return false;
    const controller = new AbortController();
    command.current = controller;
    setBusy(true);
    setProblem("");
    try {
      await action(controller.signal);
      return !controller.signal.aborted;
    } catch (error) {
      if (!controller.signal.aborted) setProblem(chatError(error, fallback));
      return false;
    } finally {
      if (command.current === controller) {
        command.current = null;
        if (!controller.signal.aborted) setBusy(false);
      }
    }
  }

  function submit(body: ChatRequest) {
    return perform(async (signal) => {
      const signature = JSON.stringify(body);
      if (pendingIntent.current?.body !== signature) {
        pendingIntent.current = { body: signature, key: crypto.randomUUID() };
      }
      try {
        const { message, run } = await postQuestion(bundle, sessionId, body, pendingIntent.current.key, signal);
        signal.throwIfAborted();
        pendingIntent.current = null;
        bundle.queryClient.setQueryData<ChatSessionDetail>(queryKeys.chat.session(sessionId), (cache) => {
          if (!cache || cache.messages.some((item) => item.id === message.id)) return cache;
          return { ...cache, messages: [...cache.messages, message] };
        });
        observe(run);
      } catch (error) {
        if (error instanceof ApiRequestError && error.status < 500) pendingIntent.current = null;
        throw error;
      }
    }, "发送失败，请重试。");
  }

  function applyStatus(status: ChatRun, stopping: boolean) {
    const done = !runIsActive(status);
    if (done && streaming.current?.id === status.id) streaming.current.controller.abort();
    setCurrent((view) => {
      const previous = view?.run.id === status.id ? view : viewRun(status);
      if (!done && !runIsActive(previous.run)) return previous;
      return { ...previous, run: status, stopping: !done && (previous.stopping || stopping) };
    });
    if (done) refreshHistory();
  }

  return {
    current,
    busy,
    problem,
    submit,
    stop: (run: ChatRun) => perform(async (signal) => {
      const status = await changeRun(bundle, run, "cancel", signal);
      signal.throwIfAborted();
      applyStatus(status, true);
    }, "停止失败，请查询运行状态后重试。"),
    retry: (run: ChatRun) => perform(async (signal) => {
      const status = await changeRun(bundle, run, "retry", signal);
      signal.throwIfAborted();
      observe(status);
    }, "重试失败，请稍后再试。"),
    refresh: (run: ChatRun) => perform(async (signal) => {
      const status = await readRun(bundle, sessionId, run.id, signal);
      signal.throwIfAborted();
      applyStatus(status, false);
    }, "暂时无法读取运行状态。"),
  };
}
