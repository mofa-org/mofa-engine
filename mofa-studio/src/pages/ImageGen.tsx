import { useMemo, useState } from "react";
import { Button, Input, Select, Card, Typography, Tag, Space, Alert, Spin, Image } from "antd";
import { PictureOutlined } from "@ant-design/icons";
import { generateImage, assetSrc } from "../services/engine";
import { useStudio } from "../store";

const SIZES = ["1024x1024", "512x512", "1792x1024", "1024x1792"];

export default function ImageGen() {
  const [prompt, setPrompt] = useState("");
  const [size, setSize] = useState(SIZES[0]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const addAsset = useStudio((s) => s.addAsset);
  // Select the stable `assets` reference and derive here. Filtering *inside* the
  // Zustand selector would return a new array every render, which makes
  // useSyncExternalStore see a fresh snapshot each time → infinite render loop.
  const assets = useStudio((s) => s.assets);
  const images = useMemo(() => assets.filter((a) => a.kind === "image"), [assets]);

  async function run() {
    const text = prompt.trim();
    if (!text || busy) return;
    setBusy(true);
    setError(null);
    try {
      const result = await generateImage(text, size);
      if (!result.ok) {
        setError(result.error ?? "generation failed");
      } else {
        addAsset("image", text, result);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ padding: 24, maxWidth: 1100, margin: "0 auto" }}>
      <Typography.Title level={4}>
        <PictureOutlined /> Image generation
      </Typography.Title>
      <Space.Compact style={{ width: "100%", marginBottom: 12 }}>
        <Input
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onPressEnter={run}
          placeholder="Describe the image you want…"
          size="large"
        />
        <Select
          value={size}
          onChange={setSize}
          size="large"
          style={{ width: 150 }}
          options={SIZES.map((s) => ({ value: s, label: s }))}
        />
        <Button type="primary" size="large" loading={busy} onClick={run}>
          Generate
        </Button>
      </Space.Compact>

      {error && <Alert type="warning" showIcon message={error} style={{ marginBottom: 12 }} />}
      {busy && (
        <Card style={{ marginBottom: 16, textAlign: "center", padding: 32 }}>
          <Spin /> <span style={{ marginLeft: 8 }}>rendering…</span>
        </Card>
      )}

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))",
          gap: 16,
        }}
      >
        {images.map((a) => (
          <Card
            key={a.id}
            size="small"
            cover={<Image src={assetSrc(a.path)} alt={a.prompt} style={{ objectFit: "cover" }} />}
          >
            <Typography.Paragraph ellipsis={{ rows: 2 }} style={{ marginBottom: 6 }}>
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
