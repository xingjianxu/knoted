import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createApiClient, type components } from "@knoted/api-client";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { TasksPage } from "../../src/features/tasks/TasksPage";
import { UiProvider } from "../../src/lib/ui-provider";
import { queryKeys, type QueryClientBundle } from "../../src/lib/query";
type Task = components["schemas"]["Task"];
type Activity = components["schemas"]["Activity"];
const stamp = "2026-11-17T00:00:00Z";
const a = "00000000-0000-4000-8000-000000000001";
const b = "00000000-0000-4000-8000-000000000002";
const c = "00000000-0000-4000-8000-000000000003";
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
function harness() {
 const rows: Task[] = [a,b].map((id,i) => ({id,title:`事项${i+1}`,project_id:null,parent_id:null,description:null,status:"todo",priority:"none",start:null,due:null,version:1,archived_at:null,deleted_at:null,created_at:stamp,updated_at:stamp}));
 const activities: Activity[] = [{id:c,task_id:a,author_kind:"manual",body:"原始跟进",version:1,created_at:stamp,updated_at:stamp,deleted_at:null},{id:b,task_id:a,author_kind:"system",body:"状态变化",version:1,created_at:stamp,updated_at:stamp,deleted_at:null}];
 const requests: Request[]=[]; let reject=false; let fail=false; let failLists=false; let failActivities=false; let cursorMode=false; let activityGate: Promise<void>|undefined; let releaseActivity: (()=>void)|undefined; let release: (()=>void)|undefined; let gate: Promise<void>|undefined;
 const fetch: typeof globalThis.fetch=async(input)=>{
  const req=input as Request;requests.push(req.clone());const url=new URL(req.url);const path=url.pathname.replace("/api/v1","");
  const error=(status:number)=>json({code:"version_conflict",message:status===409?"父事项仍有未完成子事项":"暂时不可用",current_version:2,request_id:a},status);
  if(path==="/projects")return json({items:[],next_cursor:null});
  if(path==="/tasks" && failLists)return error(503);
  if(path==="/tasks")return json({items:rows.filter(v=>!url.searchParams.has("parent_id")||v.parent_id===url.searchParams.get("parent_id")),next_cursor:null});
  if(path.includes("/activities")){
   if(path.endsWith("/revisions"))return json({items:url.searchParams.has("cursor")?[]:[{id:a,activity_id:c,author_kind:"manual",body:"历史正文",version:1,created_at:stamp}],next_cursor:cursorMode&&!url.searchParams.has("cursor")?"opaque-revisions":null});
   const item=activities.find(v=>v.id===path.split("/")[4]);
   if(req.method==="GET"){if(failActivities)return error(503);if(activityGate&&path.split("/")[2]===a)await activityGate;return json({items:cursorMode&&url.searchParams.has("cursor")?[]:activities.filter(v=>v.task_id===path.split("/")[2]),next_cursor:cursorMode&&!url.searchParams.has("cursor")?"opaque-activities":null});}
   if(fail)return error(503);
   if(req.method==="POST"){const body=await req.json() as {body:string};activities.unshift({...activities[0]!,id:a,body:body.body});return json(activities[0],201);}
   const expected=req.method==="DELETE"?Number(url.searchParams.get("expected_version")):(await req.clone().json() as {expected_version:number}).expected_version;
   if(expected!==item?.version)return error(409);
   if(req.method==="DELETE"){item!.deleted_at=stamp;item!.body="";item!.version++;return new Response(null,{status:204});}
   item!.body=(await req.json() as {body:string}).body;item!.version++;return json(item);
  }
  const row=rows.find(v=>v.id===path.split("/")[2]);if(req.method==="GET")return json(row);
  if(gate)await gate;if(reject)return error(409);Object.assign(row!,await req.json());row!.version++;return json(row);
 };
 const queryClient=new QueryClient({defaultOptions:{queries:{retry:false}}});
 const bundle={queryClient,api:createApiClient({baseUrl:"http://localhost/api/v1",fetch})} satisfies QueryClientBundle;
 return {bundle,rows,activities,requests,failActivities:()=>{failActivities=true;},failLists:()=>{failLists=true;},cursorMode:()=>{cursorMode=true;},pauseActivity:()=>{activityGate=new Promise<void>(resolve=>{releaseActivity=resolve;});},releaseActivity:()=>releaseActivity?.(),reject:()=>{reject=true;},fail:()=>{fail=true;},recover:()=>{fail=false;},pause:()=>{gate=new Promise<void>(resolve=>{release=resolve;});},release:()=>release?.()};
}
function Page({bundle}:{bundle:QueryClientBundle}){const[id,setId]=useState<string>();return <TasksPage bundle={bundle} taskId={id} onSelectTask={id=>setId(id??undefined)}/>;}
function mount(bundle:QueryClientBundle){return render(<UiProvider><QueryClientProvider client={bundle.queryClient}><Page bundle={bundle}/></QueryClientProvider></UiProvider>);}
async function board(){fireEvent.click(await screen.findByRole("button",{name:"看板视图"}));return screen.findByRole("region",{name:"事项看板"});}
async function timeline(){fireEvent.click(await screen.findByRole("button",{name:"打开事项 事项1"}));return screen.findByRole("region",{name:"跟进时间线"});}
describe("T40 看板和时间线",()=>{
 it("t40_1 父状态门槛409先乐观移动再回滚原列",async()=>{
  const h=harness();h.rows[1]!.parent_id=a;h.reject();h.pause();mount(h.bundle);await board();
  const todo=screen.getByRole("list",{name:"待办事项"});const done=screen.getByRole("list",{name:"已完成事项"});
  const card=within(todo).getByText("事项1").closest("li")!;const dataTransfer={setData:()=>{},getData:()=>a};
  fireEvent.dragStart(card,{dataTransfer});fireEvent.dragOver(done,{dataTransfer});fireEvent.drop(done,{dataTransfer});
  await waitFor(()=>expect(within(done).getByText("事项1")).toBeInTheDocument());
  expect(screen.getByRole("button",{name:"列表视图"})).toBeDisabled();await act(async()=>h.release());
  expect(await screen.findByText(/父事项仍有未完成子事项/)).toBeInTheDocument();
  expect(within(todo).getByText("事项1")).toBeInTheDocument();expect(within(done).queryByText("事项1")).not.toBeInTheDocument();
 });
 it("t40_2 删除显示标记不泄露旧正文，系统事件只读，修订显式加载",async()=>{
  const h=harness();mount(h.bundle);const line=within(await timeline());await line.findByText("原始跟进");
  expect(line.getAllByRole("button",{name:"编辑跟进"})).toHaveLength(1);
  fireEvent.click(line.getByRole("button",{name:"删除跟进"}));fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button",{name:"确认删除跟进"}));
  expect(await line.findByText("此跟进已删除")).toBeInTheDocument();expect(line.queryByText("原始跟进")).not.toBeInTheDocument();expect(line.queryByText("历史正文")).not.toBeInTheDocument();
  fireEvent.click(line.getByRole("button",{name:"查看修订"}));expect(await line.findByText("历史正文")).toBeInTheDocument();
 });
 it("t40_3 键盘状态命令可更新事项且使用CAS版本",async()=>{
  const h=harness();mount(h.bundle);await board();const card=within(screen.getByRole("list",{name:"待办事项"})).getByText("事项1").closest("li")!;
  fireEvent.keyDown(card,{key:"ArrowRight",altKey:true});expect(await within(screen.getByRole("list",{name:"进行中事项"})).findByText("事项1")).toBeInTheDocument();
  await waitFor(()=>expect(h.rows[0]!.status).toBe("in_progress"));expect(await h.requests.find(v=>v.method==="PATCH")?.json()).toEqual({status:"in_progress",expected_version:1});
 });
 it("失败保留输入可重试，Query刷新不提升编辑草稿CAS，取消和切事项清除草稿",async()=>{
  const h=harness();mount(h.bundle);const line=within(await timeline());
  fireEvent.change(line.getByLabelText("新增跟进正文"),{target:{value:"新增草稿"}});h.fail();fireEvent.click(line.getByRole("button",{name:"添加跟进"}));await line.findByText(/暂时不可用/);
  expect(line.getByLabelText("新增跟进正文")).toHaveValue("新增草稿");h.recover();fireEvent.click(line.getByRole("button",{name:"添加跟进"}));await waitFor(()=>expect(line.getByLabelText("新增跟进正文")).toHaveValue(""));
  fireEvent.click(line.getAllByRole("button",{name:"编辑跟进"})[1]!);fireEvent.change(line.getByLabelText("编辑跟进正文"),{target:{value:"我的编辑"}});h.activities.find(v=>v.id===c)!.version=2;
  await act(async()=>{await h.bundle.queryClient.invalidateQueries({queryKey:queryKeys.tasks.all()});});
  fireEvent.click(line.getByRole("button",{name:"保存跟进"}));expect(await line.findByText(/草稿已保留/)).toBeInTheDocument();expect(line.getByLabelText("编辑跟进正文")).toHaveValue("我的编辑");expect(line.getByRole("button",{name:"保存跟进"})).toBeDisabled();
  fireEvent.click(line.getByRole("button",{name:"取消编辑跟进"}));expect(line.queryByLabelText("编辑跟进正文")).not.toBeInTheDocument();fireEvent.click(screen.getByRole("button",{name:"打开事项 事项2"}));expect(await screen.findByRole("region",{name:"跟进时间线"})).not.toHaveTextContent("原始跟进");
 });
 it("状态保存成功但列表刷新失败时仍显示已确认的新列",async()=>{
  const h=harness();mount(h.bundle);await board();h.failLists();
  const card=within(screen.getByRole("list",{name:"待办事项"})).getByText("事项1").closest("li")!;
  fireEvent.keyDown(card,{key:"ArrowRight",altKey:true});
  await screen.findAllByText("暂时不可用");
  expect(within(screen.getByRole("list",{name:"进行中事项"})).getByText("事项1")).toBeInTheDocument();
 });
 it("事项跟进游标加载禁用翻页，切换事项后迟到响应不串内容",async()=>{
  const h=harness();h.cursorMode();mount(h.bundle);const line=within(await timeline());await line.findByText("原始跟进");
  h.pauseActivity();fireEvent.click(line.getByRole("button",{name:"下一页跟进"}));
  await waitFor(()=>expect(line.getByRole("button",{name:"上一页跟进"})).toBeDisabled());
  expect(line.getByRole("button",{name:"下一页跟进"})).toBeDisabled();
  fireEvent.click(screen.getByRole("button",{name:"打开事项 事项2"}));
  const second=within(await screen.findByRole("region",{name:"跟进时间线"}));await second.findByText("暂无跟进记录");
  await act(async()=>h.releaseActivity());expect(second.queryByText("原始跟进")).not.toBeInTheDocument();
  expect(h.requests.some(r=>new URL(r.url).searchParams.get("cursor")==="opaque-activities")).toBe(true);
 });
 it("归档事项看板不可拖动/键盘更改，时间线禁止新增和编辑但可读取",async()=>{
  const h=harness();h.rows[0]!.archived_at=stamp;mount(h.bundle);await board();
  const card=within(screen.getByRole("list",{name:"待办事项"})).getByText("事项1").closest("li")!;
  expect(card).toHaveAttribute("draggable","false");fireEvent.keyDown(card,{key:"ArrowRight",altKey:true});
  expect(h.requests.some(r=>r.method==="PATCH")).toBe(false);
  const line=within(await timeline());await line.findByText("原始跟进");expect(line.getByRole("button",{name:"编辑跟进"})).toBeDisabled();expect(line.getByLabelText("新增跟进正文")).toBeDisabled();
 });

 it("冲突后显式刷新基础版本保留草稿，保存成功，再次编辑取消不发请求",async()=>{
  const h=harness();mount(h.bundle);const line=within(await timeline());await line.findByText("原始跟进");
  fireEvent.click(line.getByRole("button",{name:"编辑跟进"}));fireEvent.change(line.getByLabelText("编辑跟进正文"),{target:{value:"我的新正文"}});h.activities[0]!.version=2;h.activities[0]!.body="远端正文";
  fireEvent.click(line.getByRole("button",{name:"保存跟进"}));await line.findByText(/草稿已保留/);
  fireEvent.click(line.getByRole("button",{name:"重新读取跟进版本"}));
  await waitFor(()=>expect(line.getByRole("button",{name:"保存跟进"})).toBeEnabled());expect(line.getByLabelText("编辑跟进正文")).toHaveValue("我的新正文");
  fireEvent.click(line.getByRole("button",{name:"保存跟进"}));await waitFor(()=>expect(line.queryByLabelText("编辑跟进正文")).not.toBeInTheDocument());expect(line.getByText("我的新正文")).toBeInTheDocument();
  const patches=h.requests.filter(v=>v.method==="PATCH");expect(await patches[1]!.json()).toEqual({body:"我的新正文",expected_version:2});
  fireEvent.click(line.getByRole("button",{name:"编辑跟进"}));fireEvent.change(line.getByLabelText("编辑跟进正文"),{target:{value:"取消的正文"}});fireEvent.click(line.getByRole("button",{name:"取消编辑跟进"}));expect(h.requests.filter(v=>v.method==="PATCH")).toHaveLength(2);
 });
 it("修订游标保持不透明且前后翻页，删除确认可取消",async()=>{
  const h=harness();h.cursorMode();mount(h.bundle);const line=within(await timeline());await line.findByText("原始跟进");
  fireEvent.click(line.getByRole("button",{name:"查看修订"}));await line.findByText("历史正文");fireEvent.click(line.getByRole("button",{name:"下一页修订"}));await line.findByText("暂无修订");
  expect(h.requests.some(v=>new URL(v.url).searchParams.get("cursor")==="opaque-revisions")).toBe(true);fireEvent.click(line.getByRole("button",{name:"上一页修订"}));await line.findByText("历史正文");
  fireEvent.click(line.getByRole("button",{name:"删除跟进"}));fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button",{name:"取消"}));expect(h.requests.some(v=>v.method==="DELETE")).toBe(false);
 });

 it("键盘移动后保留卡片焦点，后续刷新不窃取用户选择的焦点",async()=>{
  const h=harness();mount(h.bundle);await board();
  const card=within(screen.getByRole("list",{name:"待办事项"})).getByText("事项1").closest("li")!;card.focus();fireEvent.keyDown(card,{key:"ArrowRight",altKey:true});
  await waitFor(()=>expect(screen.getByRole("button",{name:"刷新事项列表"})).toBeEnabled());
  const moved=within(screen.getByRole("list",{name:"进行中事项"})).getByText("事项1").closest("li")!;expect(moved).toHaveFocus();
  const refresh=screen.getByRole("button",{name:"刷新事项列表"});refresh.focus();fireEvent.click(refresh);
  await waitFor(()=>expect(refresh).toBeEnabled());expect(refresh).toHaveFocus();
 });

 it("DELETE成功后即使跟进刷新失败也必须隐藏旧正文并保留删除标记",async()=>{
  const h=harness();mount(h.bundle);const line=within(await timeline());await line.findByText("原始跟进");h.failActivities();
  fireEvent.click(line.getByRole("button",{name:"删除跟进"}));fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button",{name:"确认删除跟进"}));await line.findByText("暂时不可用");
  expect(line.queryByText("原始跟进")).not.toBeInTheDocument();expect(line.getByText("此跟进已删除")).toBeInTheDocument();
 });

});
