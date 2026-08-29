import { useEffect, useState } from "react";
import { Layout, Menu, Tag, Typography, Space, Tooltip } from "antd";
import {
  MessageOutlined,
  PictureOutlined,
  VideoCameraOutlined,
  ApartmentOutlined,
  AppstoreOutlined,
  SettingOutlined,
  ThunderboltFilled,
} from "@ant-design/icons";
import { Routes, Route, useNavigate, useLocation, Navigate } from "react-router-dom";
import { getCapabilities, type CapabilityRow } from "./services/engine";
import ErrorBoundary from "./ErrorBoundary";
import Chat from "./pages/Chat";
import ImageGen from "./pages/ImageGen";
import VideoGen from "./pages/VideoGen";
import Workflow from "./pages/Workflow";
import Assets from "./pages/Assets";
import Settings from "./pages/Settings";

const { Sider, Header, Content } = Layout;

const NAV = [
  { key: "/chat", icon: <MessageOutlined />, label: "Chat" },
  { key: "/image", icon: <PictureOutlined />, label: "Image" },
  { key: "/video", icon: <VideoCameraOutlined />, label: "Video" },
  { key: "/workflow", icon: <ApartmentOutlined />, label: "Workflow" },
  { key: "/assets", icon: <AppstoreOutlined />, label: "Assets" },
  { key: "/settings", icon: <SettingOutlined />, label: "Settings" },
];

/** Header chips: which capabilities are wired up right now, and on-device vs cloud. */
function CapabilityBar() {
  const [caps, setCaps] = useState<CapabilityRow[]>([]);
  useEffect(() => {
    getCapabilities().then(setCaps).catch(() => setCaps([]));
  }, []);

  const available = caps.filter((c) => c.available);
  if (available.length === 0) {
    return (
      <Tag color="default" bordered={false}>
        offline — no backend configured
      </Tag>
    );
  }
  return (
    <Space size={4} wrap>
      {available.map((c, i) => (
        <Tooltip key={i} title={`${c.provider} · ${c.local ? "on-device" : "cloud"}`}>
          <Tag color={c.local ? "green" : "geekblue"} bordered={false}>
            {c.capability}
          </Tag>
        </Tooltip>
      ))}
    </Space>
  );
}

export default function App() {
  const navigate = useNavigate();
  const location = useLocation();
  const selected = "/" + (location.pathname.split("/")[1] || "chat");

  return (
    <Layout style={{ height: "100vh" }}>
      <Sider theme="dark" width={210} style={{ borderRight: "1px solid #1c1f27" }}>
        <div
          style={{
            height: 56,
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "0 18px",
            fontWeight: 700,
            fontSize: 16,
          }}
        >
          <ThunderboltFilled style={{ color: "#6d5efc" }} />
          MoFA Studio
        </div>
        <Menu
          theme="dark"
          mode="inline"
          selectedKeys={[selected]}
          items={NAV}
          onClick={({ key }) => navigate(key)}
          style={{ background: "transparent", borderInlineEnd: "none" }}
        />
      </Sider>
      <Layout>
        <Header
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            background: "#0e1116",
            borderBottom: "1px solid #1c1f27",
            paddingInline: 20,
          }}
        >
          <Typography.Text type="secondary">
            Local-first creative studio, powered by the MoFA engine
          </Typography.Text>
          <CapabilityBar />
        </Header>
        <Content style={{ overflow: "auto" }}>
          <ErrorBoundary key={selected}>
            <Routes>
              <Route path="/" element={<Navigate to="/chat" replace />} />
              <Route path="/chat" element={<Chat />} />
              <Route path="/image" element={<ImageGen />} />
              <Route path="/video" element={<VideoGen />} />
              <Route path="/workflow" element={<Workflow />} />
              <Route path="/assets" element={<Assets />} />
              <Route path="/settings" element={<Settings />} />
            </Routes>
          </ErrorBoundary>
        </Content>
      </Layout>
    </Layout>
  );
}
