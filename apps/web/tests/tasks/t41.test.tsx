import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createApiClient, type components } from "@knoted/api-client";
import { describe, expect, it, vi } from "vitest";
import { DashboardPage } from "../../src/features/dashboard/DashboardPage";
import { UiProvider } from "../../src/lib/ui-provider";

type Summary = components["schemas"]["DashboardSummary"];
const stamp = "2026-11-17T00:00:00Z";
const task: components["schemas"]["Task"] = {id:"00000000-0000-4000-8000-000000000001",title:"已完成事项",project_id:null,parent_id:null,description:null,status:"done",priority:"none",start:null,due:{date:"2020-01-01",time:null,timezone:"Asia/Shanghai"},version:1,archived_at:null,deleted_at:null,created_at:stamp,updated_at:stamp};
const empty = (): Summary => ({counts:{notes:0,tasks_open:0,tasks_due_today:0,projects_active:0,files:0},tasks_by_status:{todo:0,in_progress:0,done:0,cancelled:0},recent_tasks:[],recent_notes:[],due_tasks:[],job_failures:0,indexing_enabled:false});
function harness(summary = empty()) {
 const requests: Request[]=[]; let fail=false; let gate: Promise<void>|undefined; let release:(()=>void)|undefined;
 const bundle={queryClient:new QueryClient({defaultOptions:{queries:{retry:false,staleTime:30000}}}),api:createApiClient({baseUrl:"http://localhost/api/v1",fetch:async input=>{const req=input as Request;requests.push(req);if(gate)await gate;return new Response(JSON.stringify(fail?{code:"dependency_unavailable",message:"SQL secret",request_id:"x"}:summary),{status:fail?503:200,headers:{"content-type":"application/json"}});}})};
 const note=vi.fn();const select=vi.fn();
 const mount=()=>render(<UiProvider><QueryClientProvider client={bundle.queryClient}><DashboardPage bundle={bundle} onSelectNote={note} onSelectTask={select}/></QueryClientProvider></UiProvider>);
 return {mount,requests,note,select,summary,fail:()=>{fail=true;},recover:()=>{fail=false;},pause:()=>{gate=new Promise<void>(r=>{release=r;});},release:()=>release?.()};
}
describe("T41 工作概览",()=>{
 it("t41_1 已完成事项在最近事项中可见但不显示逾期",async()=>{const s=empty();s.recent_tasks=[task];s.tasks_by_status.done=1;const h=harness(s);h.mount();fireEvent.click(await screen.findByRole("button",{name:"打开事项 已完成事项"}));expect(h.select).toHaveBeenCalledWith(task.id);expect(screen.queryByText("已逾期")).not.toBeInTheDocument();expect(screen.getByText("暂无到期事项")).toBeInTheDocument();});
 it("t41_2 零数据空态并明确索引和文件浏览解析尚未开放",async()=>{harness().mount();expect(await screen.findByText("暂无最近笔记")).toBeInTheDocument();expect(screen.getByText("暂无到期事项")).toBeInTheDocument();expect(screen.getByText(/索引未启用/)).toBeInTheDocument();expect(screen.getByText(/文件浏览与解析暂未开放/)).toBeInTheDocument();});
 it("t41_3 只读取本站概览，不发送任何外部通知或写请求",async()=>{const h=harness();h.mount();await screen.findByText("暂无最近笔记");expect(h.requests.map(r=>[r.method,r.url])).toEqual([["GET","http://localhost/api/v1/dashboard"]]);});
 it("展示到期和失败任务，笔记导航使用真实ID",async()=>{const s=empty();s.due_tasks=[{...task,title:"待处理",status:"todo"}];s.counts.tasks_due_today=12;s.job_failures=3;s.recent_notes=[{id:task.id,title:"最近笔记",parent_id:null,document:{blocks:[]},position:0,format_version:1,content_version:1,deleted_at:null,created_at:stamp,updated_at:stamp}];const h=harness(s);h.mount();expect(await screen.findByText("已逾期")).toBeInTheDocument();expect(screen.getByText(/失败任务：3/)).toBeInTheDocument();expect(screen.getByText(/今日到期：12/)).toBeInTheDocument();fireEvent.click(screen.getByRole("button",{name:"打开笔记 最近笔记"}));expect(h.note).toHaveBeenCalledWith(task.id);});
 it("精确截止已到才逾期，今天日期型尚未越过上海午夜",async()=>{
  const now=vi.spyOn(Date,"now").mockReturnValue(Date.parse("2026-11-17T02:30:00Z"));
  try {const s=empty();s.due_tasks=[{...task,title:"刚到截止",status:"todo",due:{date:"2026-11-17",time:"10:30",timezone:"Asia/Shanghai"}},{...task,id:"00000000-0000-4000-8000-000000000002",title:"今日整天",status:"todo",due:{date:"2026-11-17",time:null,timezone:"Asia/Shanghai"}}];harness(s).mount();await screen.findByText("刚到截止");expect(screen.getAllByText("已逾期")).toHaveLength(1);expect(screen.getAllByText("今日到期")).toHaveLength(1);} finally {now.mockRestore();}
 });
 it("失败显示安全反馈并可重试",async()=>{const h=harness();h.fail();h.mount();expect(await screen.findByRole("alert")).not.toHaveTextContent("SQL secret");h.recover();fireEvent.click(screen.getByRole("button",{name:"重试"}));expect(await screen.findByText("暂无最近笔记")).toBeInTheDocument();});
 it("重新进入必须刷新，加载期间隐藏旧到期数据",async()=>{const s=empty();s.due_tasks=[{...task,title:"旧到期",status:"todo"}];const h=harness(s);const page=h.mount();await screen.findByText("旧到期");page.unmount();s.due_tasks=[];h.pause();h.mount();expect(screen.queryByText("旧到期")).not.toBeInTheDocument();await act(async()=>h.release());await screen.findByText("暂无到期事项");await waitFor(()=>expect(h.requests).toHaveLength(2));});
});
