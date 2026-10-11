import React from "react";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "@/components/theme-provider";
import { TooltipProvider } from "@/components/ui/tooltip";
import { EpisodeWindow } from "./episode-window";
import "@/index.css";

declare global {
  interface Window {
    /** The episode this window shows; set by the backend's initialization
     *  script when it opens the window (`open_episode_window`). */
    __EPISODE_FILE__?: string;
  }
}

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("Root element #root not found");

const file = typeof window.__EPISODE_FILE__ === "string" ? window.__EPISODE_FILE__ : "";

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    {/* Same props as the main window, so both follow one stored theme. */}
    <ThemeProvider
      attribute="class"
      defaultTheme="dark"
      enableSystem
      disableTransitionOnChange
      storageKey="polygluttony-theme"
    >
      <TooltipProvider>
        {file ? (
          <EpisodeWindow file={file} />
        ) : (
          <div className="flex h-screen items-center justify-center bg-background px-8 text-center text-[12.5px] text-muted-foreground">
            This window doesn’t know which episode to show. Close it and open the episode again from
            Translate.
          </div>
        )}
      </TooltipProvider>
    </ThemeProvider>
  </React.StrictMode>,
);
