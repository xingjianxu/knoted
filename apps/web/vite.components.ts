import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";

// Component fixtures need the same React and styles as the app, without routing.
export function componentPlugins() {
  return [react(), tailwindcss()];
}
