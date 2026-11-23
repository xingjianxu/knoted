import { fireEvent, screen } from "@testing-library/react";

export async function openWorkspaceLogout() {
  const account = await screen.findByRole("button", { name: "账号菜单" });
  if (account.getAttribute("aria-expanded") !== "true") fireEvent.click(account);
  return screen.findByRole("menuitem", { name: "退出登录" });
}
