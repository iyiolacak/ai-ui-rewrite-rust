// Upstream's default thread (templates/minimal, MIT) on the Rust example's demo
// model, so screenshots of the same scenes can be compared side by side.
// Scenes: ?scene=empty | conversation | stream (with &at=MS for mid-stream).
import {
  AssistantRuntimeProvider,
  useLocalRuntime,
  useAui,
  type ChatModelAdapter,
} from "@assistant-ui/react";
import { useEffect, type FC } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "./fonts.css";
import "./globals.css";
import { Thread } from "@/components/assistant-ui/elements/thread.aui";
import { TooltipProvider } from "@/components/ui/tooltip";

const params = new URLSearchParams(location.search);
const scene = params.get("scene") ?? "empty";
const DELAY_MS = scene === "stream" ? 18 : 0;

// examples/thread/src/demo.rs answer()
const answer = (topic: string) =>
  `Here's a quick answer about **${topic}**.\n\n` +
  "This reply comes from the example's local demo model, streamed token by token through " +
  "the Rust port of assistant-ui's runtime.\n\n" +
  "- Edit a message or regenerate an answer to create **branches**\n" +
  "- Every update is a cumulative `ChatModelRunResult`, as upstream\n" +
  "- Stop the stream at any time\n\n" +
  "Try asking for the **weather**, or to send an **email**, to see tools and approvals.\n\n" +
  "```rust\n" +
  "let runtime = LocalThreadRuntime::new(adapter, LocalRuntimeOptions::default());\n" +
  `slint::spawn_local(runtime.append(AppendMessage::user_text(None, "${topic}"))?)?;\n` +
  "```\n\n" +
  "| Layer | Crate |\n|---|---|\n| Runtime | `ai-ui-core` |\n| Protocols | `ai-ui-stream` |\n| UI | `ai-ui-slint` |\n";

const tokens = (text: string) => {
  const out: string[] = [];
  let current = "";
  for (const c of text) {
    current += c;
    if (/\s/.test(c)) {
      out.push(current);
      current = "";
    }
  }
  if (current) out.push(current);
  return out;
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const demoModel: ChatModelAdapter = {
  async *run({ messages, abortSignal }) {
    const last = messages[messages.length - 1];
    const prompt = last?.content.map((p) => (p.type === "text" ? p.text : "")).join("") ?? "";
    const topic = prompt.trim().replace(/\?+$/, "");
    const reasoning = tokens(
      `The user asked about '${topic}'. A short answer with a list, code and a table shows the renderer.`,
    );
    const body = tokens(answer(topic));
    const total = reasoning.length + body.length;
    for (let emitted = DELAY_MS ? 1 : total; emitted <= total; emitted++) {
      if (DELAY_MS) await sleep(DELAY_MS);
      if (abortSignal.aborted) return;
      const r = Math.min(emitted, reasoning.length);
      const b = Math.max(0, emitted - reasoning.length);
      yield {
        content: [
          { type: "reasoning", text: reasoning.slice(0, r).join("") },
          ...(b > 0 ? [{ type: "text" as const, text: body.slice(0, b).join("") }] : []),
        ],
      };
    }
  },
};

// Send the scene's prompts, then mark the page ready for a screenshot.
const Scene: FC = () => {
  const aui = useAui();
  useEffect(() => {
    const w = window as unknown as { __ready?: boolean };
    const prompts =
      scene === "conversation" ? ["What can you do?", "Show me a table"] : scene === "stream" ? ["Show me a table"] : [];
    (async () => {
      for (const text of prompts) {
        aui.thread().append(text);
        if (scene === "stream") break;
        await sleep(50);
        while (aui.thread().getState().isRunning) await sleep(20);
      }
      w.__ready = true;
    })();
  }, [aui]);
  return null;
};

const App: FC = () => {
  const runtime = useLocalRuntime(demoModel);
  return (
    <TooltipProvider>
      <AssistantRuntimeProvider runtime={runtime}>
        <div className="h-dvh">
          <Thread />
        </div>
        <Scene />
      </AssistantRuntimeProvider>
    </TooltipProvider>
  );
};

createRoot(document.getElementById("root")!).render(<App />);
