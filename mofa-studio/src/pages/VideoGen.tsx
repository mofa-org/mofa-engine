import { useMemo, useState } from "react";
import { Button, Input, Select, Card, Typography, Tag, Space, Alert, Progress } from "antd";
import { VideoCameraOutlined } from "@ant-design/icons";
import { generateVideo, assetSrc } from "../services/engine";
import { useStudio } from "../store";

const RESOLUTIONS = ["720p", "1080p", "480p"];
const RATIOS = ["16:9", "9:16", "1:1"];

export default function VideoGen() {
  const [prompt, setPrompt] = useState("");
  const [resolution, setResolution] = useState(RESOLUTIONS[0]);
  const [ratio, setRatio] = useState(RATIOS[0]);
  const [duration, setDuration] = useState(5);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const addAsset = useStudio((s) => s.addAsset);
  // Derive from the stable `assets` reference (see ImageGen): filtering inside the
  // selector would loop forever under Zustand v5 + useSyncExternalStore.
  const assets = useStudio((s) => s.assets);
  const videos = useMemo(() => assets.filter((a) => a.kind === "video"), [assets]);

  async function run() {
    const text = prompt.trim();
    if (!text || busy) return;
    setBusy(true);
    setError(null);
    try {
      const result = await generateVideo(text, { resolution, ratio, duration });
      if (!result.ok) setError(result.error ?? "generation failed");
      else addAsset("video", text, result);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ padding: 24, maxWidth: 1100, margin: "0 auto" }}>
      <Typography.Title level={4}>
        <VideoCameraOutlined /> Video generation
      </Typography.Title>
      <Space.Compact style={{ width: "100%", marginBottom: 12 }}>
        <Input
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onPressEnter={run}
          placeholder="Describe the shot… (cloud render, can take a minute)"
          size="large"
        />
        <Select
          value={resolution}
          onChange={setResolution}
          size="large"
          style={{ width: 110 }}
          options={RESOLUTIONS.map((r) => ({ value: r, label: r }))}
        />
        <Select
          value={ratio}
          onChange={setRatio}
          size="large"
          style={{ width: 100 }}
          options={RATIOS.map((r) => ({ value: r, label: r }))}
        />
        <Select
          value={duration}
          onChange={setDuration}
          size="large"
          style={{ width: 90 }}
          options={[3, 5, 8, 10].map((d) => ({ value: d, label: `${d}s` }))}
        />
        <Button type="primary" size="large" loading={busy} onClick={run}>
          Generate
        </Button>
      </Space.Compact>

      {error && <Alert type="warning" showIcon message={error} style={{ marginBottom: 12 }} />}
      {busy && (
        <Card style={{ marginBottom: 16 }}>
          <Typography.Text type="secondary">
            Rendering a clip server-side. This is a poll-until-done task and may take a minute.
          </Typography.Text>
          <Progress percent={100} status="active" showInfo={false} style={{ marginTop: 8 }} />
        </Card>
      )}

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))",
          gap: 16,
        }}
      >
        {videos.map((a) => (
          <Card key={a.id} size="small">
            <video
              src={assetSrc(a.path)}
              controls
              style={{ width: "100%", borderRadius: 8, background: "#000" }}
            />
            <Typography.Paragraph ellipsis={{ rows: 2 }} style={{ margin: "8px 0 6px" }}>
              {a.prompt}
            </Typography.Paragraph>
            <Space size={6} wrap>
              {a.provider && (
                <Tag bordered={false} color={a.local ? "green" : "geekblue"}>
                  {a.local ? "on-device" : a.provider}
                </Tag>
              )}
              <Typography.Text type="secondary" style={{ fontSize: 11 }}>
                {a.costUsd ? `$${a.costUsd.toFixed(4)}` : "free"}
                {a.durationMs ? ` · ${(a.durationMs / 1000).toFixed(1)}s` : ""}
              </Typography.Text>
            </Space>
          </Card>
        ))}
      </div>
    </div>
  );
}
