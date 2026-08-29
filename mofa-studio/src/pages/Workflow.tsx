import { useCallback } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  Handle,
  Position,
  addEdge,
  useNodesState,
  useEdgesState,
  useReactFlow,
  type Node,
  type Edge,
  type Connection,
  type NodeProps,
} from "@xyflow/react";
import { Input, Button, Typography, Tag, Spin } from "antd";
import { generateImage, generateVideo, assetSrc } from "../services/engine";
import { useStudio } from "../store";

// ComfyUI-style graph: a Prompt node feeds generator nodes. Each generator reads its
// prompt from the connected Prompt (or its own fallback), runs the engine capability,
// renders the result inline, and records it to the asset library — edges carry data.

/** Follow an incoming edge back to the Prompt node feeding `nodeId`, if any. */
function usePromptFor() {
  const rf = useReactFlow();
  return (nodeId: string, fallback: string) => {
    const edge = rf.getEdges().find((e) => e.target === nodeId);
    if (!edge) return fallback;
    const src = rf.getNode(edge.source);
    const text = (src?.data as { text?: string })?.text;
    return (text && text.trim()) || fallback;
  };
}

function PromptNode({ id, data }: NodeProps) {
  const rf = useReactFlow();
  const text = (data as { text?: string }).text ?? "";
  return (
    <div style={nodeStyle}>
      <div style={nodeTitle}>Prompt</div>
      <Input.TextArea
        value={text}
        onChange={(e) => rf.updateNodeData(id, { text: e.target.value })}
        autoSize={{ minRows: 3, maxRows: 8 }}
        placeholder="Describe what to create…"
        className="nodrag"
      />
      <Handle type="source" position={Position.Right} />
    </div>
  );
}

function GeneratorNode({ id, data, kind }: NodeProps & { kind: "image" | "video" }) {
  const rf = useReactFlow();
  const promptFor = usePromptFor();
  const addAsset = useStudio((s) => s.addAsset);
  const d = data as { busy?: boolean; path?: string; error?: string; fallbackPrompt?: string };

  const run = useCallback(async () => {
    const prompt = promptFor(id, d.fallbackPrompt ?? "");
    if (!prompt.trim()) {
      rf.updateNodeData(id, { error: "connect a Prompt node or type one" });
      return;
    }
    rf.updateNodeData(id, { busy: true, error: undefined });
    try {
      const result = kind === "image" ? await generateImage(prompt) : await generateVideo(prompt);
      if (!result.ok) {
        rf.updateNodeData(id, { busy: false, error: result.error ?? "failed" });
      } else {
        addAsset(kind, prompt, result);
        rf.updateNodeData(id, { busy: false, path: result.path, error: undefined });
      }
    } catch (e) {
      rf.updateNodeData(id, { busy: false, error: String(e) });
    }
  }, [id, d.fallbackPrompt, kind, promptFor, rf, addAsset]);

  return (
    <div style={nodeStyle}>
      <Handle type="target" position={Position.Left} />
      <div style={nodeTitle}>
        {kind === "image" ? "Image" : "Video"} <Tag bordered={false}>{kind}_gen</Tag>
      </div>
      {d.path ? (
        kind === "image" ? (
          <img src={assetSrc(d.path)} style={{ width: "100%", borderRadius: 6 }} />
        ) : (
          <video src={assetSrc(d.path)} controls style={{ width: "100%", borderRadius: 6, background: "#000" }} />
        )
      ) : (
        <div style={{ height: 90, display: "grid", placeItems: "center", color: "#666" }}>
          {d.busy ? <Spin /> : "no output yet"}
        </div>
      )}
      {d.error && (
        <Typography.Text type="danger" style={{ fontSize: 11 }}>
          {d.error}
        </Typography.Text>
      )}
      <Button size="small" type="primary" onClick={run} loading={d.busy} className="nodrag" block>
        Run
      </Button>
    </div>
  );
}

const nodeTypes = {
  prompt: PromptNode,
  image: (p: NodeProps) => <GeneratorNode {...p} kind="image" />,
  video: (p: NodeProps) => <GeneratorNode {...p} kind="video" />,
};

const initialNodes: Node[] = [
  { id: "p1", type: "prompt", position: { x: 40, y: 120 }, data: { text: "a neon city skyline at night, cinematic" } },
  { id: "i1", type: "image", position: { x: 380, y: 40 }, data: {} },
  { id: "v1", type: "video", position: { x: 380, y: 240 }, data: {} },
];
const initialEdges: Edge[] = [
  { id: "e-p1-i1", source: "p1", target: "i1", animated: true },
  { id: "e-p1-v1", source: "p1", target: "v1", animated: true },
];

function Canvas() {
  const [nodes, , onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const onConnect = useCallback(
    (c: Connection) => setEdges((eds) => addEdge({ ...c, animated: true }, eds)),
    [setEdges],
  );

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onConnect={onConnect}
      nodeTypes={nodeTypes}
      fitView
      colorMode="dark"
    >
      <Background />
      <Controls />
    </ReactFlow>
  );
}

export default function Workflow() {
  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <div style={{ padding: "12px 24px", borderBottom: "1px solid #1c1f27" }}>
        <Typography.Text strong>Workflow canvas</Typography.Text>{" "}
        <Typography.Text type="secondary">
          — connect a Prompt to a generator and hit Run. Drag from a handle to wire new nodes.
        </Typography.Text>
      </div>
      <div style={{ flex: 1 }}>
        <ReactFlowProvider>
          <Canvas />
        </ReactFlowProvider>
      </div>
    </div>
  );
}

const nodeStyle: React.CSSProperties = {
  width: 220,
  background: "#12151c",
  border: "1px solid #2a2f3a",
  borderRadius: 10,
  padding: 10,
  display: "flex",
  flexDirection: "column",
  gap: 8,
};
const nodeTitle: React.CSSProperties = { fontWeight: 600, fontSize: 13 };
