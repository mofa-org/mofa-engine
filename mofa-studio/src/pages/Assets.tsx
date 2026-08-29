import { useState } from "react";
import { Input, Card, Typography, Tag, Space, Button, Empty, Popconfirm, Image } from "antd";
import { DeleteOutlined, AppstoreOutlined } from "@ant-design/icons";
import { assetSrc } from "../services/engine";
import { useStudio } from "../store";

export default function Assets() {
  const [q, setQ] = useState("");
  const assets = useStudio((s) => s.assets);
  const removeAsset = useStudio((s) => s.removeAsset);
  const clearAssets = useStudio((s) => s.clearAssets);

  const filtered = assets.filter((a) => a.prompt.toLowerCase().includes(q.toLowerCase().trim()));

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: "0 auto" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <Typography.Title level={4} style={{ margin: 0 }}>
          <AppstoreOutlined /> Asset library
        </Typography.Title>
        {assets.length > 0 && (
          <Popconfirm title="Clear the whole library?" onConfirm={clearAssets}>
            <Button danger size="small">
              Clear all
            </Button>
          </Popconfirm>
        )}
      </div>

      <Input.Search
        placeholder="Search by prompt…"
        allowClear
        value={q}
        onChange={(e) => setQ(e.target.value)}
        style={{ maxWidth: 420, marginBottom: 20 }}
      />

      {filtered.length === 0 ? (
        <Empty style={{ marginTop: 80 }} description="No assets yet. Generate an image or video." />
      ) : (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))",
            gap: 16,
          }}
        >
          {filtered.map((a) => (
            <Card
              key={a.id}
              size="small"
              cover={
                a.kind === "image" ? (
                  <Image src={assetSrc(a.path)} alt={a.prompt} style={{ objectFit: "cover", height: 180 }} />
                ) : (
                  <video src={assetSrc(a.path)} controls style={{ width: "100%", height: 180, background: "#000" }} />
                )
              }
              actions={[
                <Popconfirm key="del" title="Remove from library?" onConfirm={() => removeAsset(a.id)}>
                  <DeleteOutlined />
                </Popconfirm>,
              ]}
            >
              <Typography.Paragraph ellipsis={{ rows: 2 }} style={{ marginBottom: 6 }}>
                {a.prompt}
              </Typography.Paragraph>
              <Space size={6} wrap>
                <Tag bordered={false} color={a.kind === "image" ? "purple" : "magenta"}>
                  {a.kind}
                </Tag>
                {a.provider && (
                  <Tag bordered={false} color={a.local ? "green" : "geekblue"}>
                    {a.local ? "on-device" : a.provider}
                  </Tag>
                )}
                <Typography.Text type="secondary" style={{ fontSize: 11 }}>
                  {a.costUsd ? `$${a.costUsd.toFixed(4)}` : "free"}
                </Typography.Text>
              </Space>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
