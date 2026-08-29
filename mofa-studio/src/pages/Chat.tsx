import { useRef, useState } from "react";
import { Button, Input, Typography, Tag, Space, Empty } from "antd";
import { SendOutlined, LoadingOutlined } from "@ant-design/icons";
import { chatStream, type ChatMessage, type StreamChunk } from "../services/engine";

type Turn = ChatMessage & {
  reasoning?: string;
  meta?: { provider: string; tokens?: number | null; cost?: number | null; ms: number };
  error?: string;
};

// The conversation opens with a system prompt the model never sees as a bubble.
const SYSTEM: ChatMessage = {
  role: "system",
  content: "You are MoFA Studio's helpful assistant. Be concise and practical.",
};

export default function Chat() {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);

  const scrollDown = () =>
    requestAnimationFrame(() => {
      scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
    });

  async function send() {
    const text = input.trim();
    if (!text || busy) return;
    setInput("");
    setBusy(true);

    const history: Turn[] = [...turns, { role: "user", content: text }];
    // The assistant bubble is appended empty and filled in as chunks stream in.
    setTurns([...history, { role: "assistant", content: "" }]);
    scrollDown();

    const wire: ChatMessage[] = [SYSTEM, ...history.map((t) => ({ role: t.role, content: t.content }))];
    const idx = history.length; // index of the assistant turn

    const patch = (fn: (t: Turn) => Turn) =>
      setTurns((prev) => prev.map((t, i) => (i === idx ? fn(t) : t)));

    try {
      await chatStream(wire, (chunk: StreamChunk) => {
        switch (chunk.type) {
          case "text":
            patch((t) => ({ ...t, content: t.content + chunk.delta }));
            break;
          case "reasoning":
            patch((t) => ({ ...t, reasoning: (t.reasoning ?? "") + chunk.delta }));
            break;
          case "completed":
            patch((t) => ({
              ...t,
              meta: {
                provider: t.meta?.provider ?? "",
                tokens: chunk.tokens_used,
                cost: chunk.cost_usd,
                ms: chunk.duration_ms,
              },
            }));
            break;
          case "started":
            patch((t) => ({ ...t, meta: { provider: chunk.provider, ms: 0 } }));
            break;
          case "error":
            patch((t) => ({ ...t, error: chunk.message }));
            break;
        }
        scrollDown();
      });
    } catch (e) {
      patch((t) => ({ ...t, error: String(e) }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div ref={scroller} style={{ flex: 1, overflow: "auto", padding: "24px 0" }}>
        <div style={{ maxWidth: 780, margin: "0 auto", padding: "0 20px" }}>
          {turns.length === 0 ? (
            <Empty
              style={{ marginTop: 120 }}
              description="Ask anything. Replies stream in, and the thought chain shows separately."
            />
          ) : (
            turns.map((t, i) => <Bubble key={i} turn={t} streaming={busy && i === turns.length - 1} />)
          )}
        </div>
      </div>
      <div style={{ borderTop: "1px solid #1c1f27", padding: 16 }}>
        <div style={{ maxWidth: 780, margin: "0 auto", display: "flex", gap: 8 }}>
          <Input.TextArea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Message MoFA Studio…"
            autoSize={{ minRows: 1, maxRows: 6 }}
            onPressEnter={(e) => {
              if (!e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
          />
          <Button
            type="primary"
            icon={busy ? <LoadingOutlined /> : <SendOutlined />}
            disabled={busy || !input.trim()}
            onClick={send}
          >
            Send
          </Button>
        </div>
      </div>
    </div>
  );
}

function Bubble({ turn, streaming }: { turn: Turn; streaming: boolean }) {
  const isUser = turn.role === "user";
  return (
    <div style={{ display: "flex", justifyContent: isUser ? "flex-end" : "flex-start", marginBottom: 16 }}>
      <div
        style={{
          maxWidth: "80%",
          background: isUser ? "#6d5efc" : "#161a22",
          border: isUser ? "none" : "1px solid #232833",
          borderRadius: 14,
          padding: "10px 14px",
        }}
      >
        {turn.reasoning && (
          <div
            style={{
              fontSize: 12,
              color: "#8a90a2",
              borderLeft: "2px solid #333a49",
              paddingLeft: 10,
              marginBottom: 8,
              whiteSpace: "pre-wrap",
            }}
          >
            {turn.reasoning}
          </div>
        )}
        <Typography.Text style={{ whiteSpace: "pre-wrap", color: isUser ? "#fff" : "#e6e8ee" }}>
          {turn.content || (streaming ? "…" : "")}
        </Typography.Text>
        {turn.error && (
          <div style={{ marginTop: 6 }}>
            <Tag color="error" bordered={false}>
              {turn.error}
            </Tag>
          </div>
        )}
        {turn.meta && (turn.meta.tokens || turn.meta.ms) && (
          <div style={{ marginTop: 6 }}>
            <Space size={6}>
              {turn.meta.provider && (
                <Tag bordered={false} color="geekblue">
                  {turn.meta.provider}
                </Tag>
              )}
              {turn.meta.tokens ? (
                <Typography.Text type="secondary" style={{ fontSize: 11 }}>
                  {turn.meta.tokens} tok
                </Typography.Text>
              ) : null}
              <Typography.Text type="secondary" style={{ fontSize: 11 }}>
                {turn.meta.cost ? `$${turn.meta.cost.toFixed(4)}` : "free"}
              </Typography.Text>
            </Space>
          </div>
        )}
      </div>
    </div>
  );
}
