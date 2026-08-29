import { useEffect, useState } from "react";
import { Typography, Table, Tag, Button, Card, Alert, Space, Divider } from "antd";
import { ReloadOutlined, SettingOutlined } from "@ant-design/icons";
import { getCapabilities, type CapabilityRow } from "../services/engine";

export default function Settings() {
  const [caps, setCaps] = useState<CapabilityRow[]>([]);
  const [loading, setLoading] = useState(false);

  const refresh = () => {
    setLoading(true);
    getCapabilities()
      .then(setCaps)
      .catch(() => setCaps([]))
      .finally(() => setLoading(false));
  };
  useEffect(refresh, []);

  return (
    <div style={{ padding: 24, maxWidth: 900, margin: "0 auto" }}>
      <Typography.Title level={4}>
        <SettingOutlined /> Settings
      </Typography.Title>

      <Card
        title="Engine capabilities"
        extra={
          <Button icon={<ReloadOutlined />} size="small" loading={loading} onClick={refresh}>
            Refresh
          </Button>
        }
        style={{ marginBottom: 20 }}
      >
        <Table
          size="small"
          rowKey={(r) => `${r.capability}-${r.provider}`}
          pagination={false}
          dataSource={caps}
          columns={[
            { title: "Capability", dataIndex: "capability" },
            { title: "Provider", dataIndex: "provider" },
            {
              title: "Where",
              dataIndex: "local",
              render: (local: boolean) => (
                <Tag color={local ? "green" : "geekblue"} bordered={false}>
                  {local ? "on-device" : "cloud"}
                </Tag>
              ),
            },
            {
              title: "Status",
              dataIndex: "available",
              render: (ok: boolean) => (
                <Tag color={ok ? "success" : "default"} bordered={false}>
                  {ok ? "ready" : "not configured"}
                </Tag>
              ),
            },
          ]}
        />
      </Card>

      <Card title="Bring your own key (BYOK)">
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 16 }}
          message="Keys stay on your machine"
          description="The studio never uploads your keys. Providers are configured from the environment when the app launches; set a key, relaunch, and it appears above."
        />
        <Typography.Paragraph>
          <Typography.Text strong>Local chat (offline):</Typography.Text> install{" "}
          <Typography.Text code>Ollama</Typography.Text> and pull a model — it is auto-detected at{" "}
          <Typography.Text code>http://127.0.0.1:11434</Typography.Text>.
        </Typography.Paragraph>
        <Divider style={{ margin: "12px 0" }} />
        <Typography.Paragraph>
          <Typography.Text strong>Cloud chat + image + video (Agnes AI):</Typography.Text>
        </Typography.Paragraph>
        <Space direction="vertical" size={4} style={{ width: "100%" }}>
          <Typography.Text code>export AGNES_API_KEY="…"</Typography.Text>
          <Typography.Text code>export AGNES_BASE_URL="https://apihub.agnes-ai.com/v1"</Typography.Text>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            Optional model overrides: AGNES_CHAT_MODEL, AGNES_IMAGE_MODEL, AGNES_VIDEO_MODEL.
          </Typography.Text>
        </Space>
        <Alert
          type="warning"
          showIcon
          style={{ marginTop: 16 }}
          message="Runtime provider editing is coming"
          description="Adding providers from this screen (without relaunching) depends on the runtime provider-config API, which is still under review. For now, configuration is environment-based."
        />
      </Card>
    </div>
  );
}
