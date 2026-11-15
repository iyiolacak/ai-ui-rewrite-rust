// Upstream assistant-ui (React, the published packages of our reference
// commit) on the workload of `ai-ui-example-thread --bench`: a seeded history,
// then the demo model's reply streamed one word every `delay` ms. The thread
// follows upstream's default `thread.aui.tsx` structure (viewport, messages,
// grouped parts with reasoning, GFM Markdown, action bar, branch picker,
// composer) without Tailwind, so it is if anything lighter than upstream's.
import {
  ActionBarPrimitive,
  AssistantRuntimeProvider,
  BranchPickerPrimitive,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useLocalRuntime,
  type ChatModelAdapter,
  type ThreadMessageLike,
} from "@assistant-ui/react";
import { MarkdownTextPrimitive } from "@assistant-ui/react-markdown";
import remarkGfm from "remark-gfm";
import { type FC } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const params = new URLSearchParams(location.search);
const HISTORY = Number(params.get("history") ?? 1000);
const DELAY_MS = Number(params.get("delay") ?? 18);

// examples/thread/src/bench.rs REPLY
const REPLY =
  "Here's a quick answer.\n\nThis reply comes from a local model, streamed token by token.\n\n" +
  "- Edit a message or regenerate an answer\n- Every update is a cumulative result\n- Stop the stream at any time\n\n" +
  "```rust\nlet runtime = LocalThreadRuntime::new(adapter, options);\n```\n\n" +
  "| Layer | Crate |\n|---|---|\n| Runtime | core |\n| UI | slint |\n";

// bench.rs history()
const history = (n: number): ThreadMessageLike[] =>
  Array.from({ length: n }, (_, i) => {
    const user = i % 2 === 0;
    return {
      id: `h${i}`,
      role: user ? "user" : "assistant",
      createdAt: new Date(0),
      content: [{ type: "text", text: user ? "A question about the thread" : REPLY }],
      ...(user ? {} : { status: { type: "complete", reason: "stop" } }),
    } as ThreadMessageLike;
  });

// demo.rs answer()
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

// demo.rs tokens(): words with their trailing whitespace.
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

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => {
      clearTimeout(t);
      resolve();
    });
  });

// demo.rs DemoStream: reasoning first, then the answer, one token per step,
// each step a cumulative snapshot.
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
    for (let emitted = 1; emitted <= total; emitted++) {
      await sleep(DELAY_MS, abortSignal);
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

const MarkdownText: FC = () => (
  <MarkdownTextPrimitive remarkPlugins={[remarkGfm]} className="aui-md" defer />
);

const Reasoning: FC<{ text: string }> = ({ text }) => (
  <details className="reasoning">
    <summary>Reasoning</summary>
    <p>{text}</p>
  </details>
);

const BranchPicker: FC = () => (
  <BranchPickerPrimitive.Root hideWhenSingleBranch className="branch">
    <BranchPickerPrimitive.Previous>‹</BranchPickerPrimitive.Previous>
    <BranchPickerPrimitive.Number /> / <BranchPickerPrimitive.Count />
    <BranchPickerPrimitive.Next>›</BranchPickerPrimitive.Next>
  </BranchPickerPrimitive.Root>
);

const AssistantMessage: FC = () => (
  <MessagePrimitive.Root className="assistant" data-role="assistant">
    <div className="content">
      <MessagePrimitive.Parts components={{ Text: MarkdownText, Reasoning }} />
    </div>
    <div className="footer">
      <BranchPicker />
      <ActionBarPrimitive.Root hideWhenRunning autohide="not-last" className="actions">
        <ActionBarPrimitive.Copy>Copy</ActionBarPrimitive.Copy>
        <ActionBarPrimitive.Reload>Regenerate</ActionBarPrimitive.Reload>
      </ActionBarPrimitive.Root>
    </div>
  </MessagePrimitive.Root>
);

const UserMessage: FC = () => (
  <MessagePrimitive.Root className="user" data-role="user">
    <div className="bubble">
      <MessagePrimitive.Parts />
    </div>
    <ActionBarPrimitive.Root hideWhenRunning autohide="not-last" className="actions">
      <ActionBarPrimitive.Edit>Edit</ActionBarPrimitive.Edit>
    </ActionBarPrimitive.Root>
    <BranchPicker />
  </MessagePrimitive.Root>
);

const Thread: FC = () => (
  <ThreadPrimitive.Root className="thread">
    <ThreadPrimitive.Viewport className="viewport">
      <div className="column">
        <ThreadPrimitive.Messages components={{ UserMessage, AssistantMessage }} />
        <ThreadPrimitive.ViewportFooter className="composer-footer">
          <ComposerPrimitive.Root className="composer">
            <ComposerPrimitive.Input placeholder="Send a message..." rows={1} aria-label="Message input" />
            <ComposerPrimitive.Send>Send</ComposerPrimitive.Send>
          </ComposerPrimitive.Root>
        </ThreadPrimitive.ViewportFooter>
      </div>
    </ThreadPrimitive.Viewport>
  </ThreadPrimitive.Root>
);

const App: FC = () => {
  const runtime = useLocalRuntime(demoModel, { initialMessages: history(HISTORY) });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread />
    </AssistantRuntimeProvider>
  );
};

(window as unknown as { __started: number }).__started = performance.now();
createRoot(document.getElementById("root")!).render(<App />);
