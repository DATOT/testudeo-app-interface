"use client";
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Alert,
  Badge,
  Button,
  Card,
  HelperText,
  HR,
  Label,
  Progress,
  Select,
  Spinner,
  TextInput,
} from "flowbite-react";
import {
  HiOutlineWifi,
  HiOutlineRefresh,
  HiOutlineCheckCircle,
  HiOutlineExclamationCircle,
  HiOutlineEyeOff,
  HiOutlineEye,
} from "react-icons/hi";

type BleDevice = {
  id: string;
  name: string | null;
  rssi: number | null;
};

const CONNECT_TIMEOUT_MS = 12000;

export default function BleConnectPage() {
  const [devices, setDevices] = useState<BleDevice[]>([]);
  const [selectedDevice, setSelectedDevice] = useState("");
  const [isScanning, setIsScanning] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [progress, setProgress] = useState(0);
  const [statusMessage, setStatusMessage] = useState("");
  const [statusType, setStatusType] = useState<"info" | "success" | "failure">(
    "info",
  );
  const [connected, setConnected] = useState(false);
  const [showPassword, setShowPassword] = useState(false);

  const [ssid, setSsid] = useState("");
  const [password, setPassword] = useState("");

  // Tải thiết bị đã biết
  useEffect(() => {
    const load = async () => {
      try {
        const list = await invoke<BleDevice[]>("get_ble_devices");
        setDevices(list);
        if (list.length > 0) setSelectedDevice(list[0].id);
      } catch (error) {
        setStatusMessage(`Lỗi: ${error}`);
        setStatusType("failure");
      }
    };
    load();
  }, []);

  const handleScan = async () => {
    setIsScanning(true);
    setStatusMessage("Đang quét thiết bị...");
    setStatusType("info");
    try {
      const list = await invoke<BleDevice[]>("scan_ble_devices", {
        timeoutMs: 5000,
      });
      setDevices(list);
      if (list.length > 0) {
        setSelectedDevice(list[0].id);
        setStatusMessage("");
        setStatusType("success");
      } else {
        setStatusMessage("Không tìm thấy thiết bị nào.");
        setStatusType("failure");
      }
    } catch (error) {
      setStatusMessage(`Lỗi quét: ${error}`);
      setStatusType("failure");
    } finally {
      setIsScanning(false);
    }
  };

  const validate = () => {
    if (!selectedDevice) return "Hãy chọn thiết bị robot của bạn.";
    if (!ssid.trim()) return "Hãy nhập tên WiFi (SSID).";
    if (password && password.length < 8)
      return "Mật khẩu WiFi phải có ít nhất 8 ký tự.";
    return null;
  };

  // Kết nối + gửi WiFi trong một lần
  const handleConnectAndSend = async (e: React.FormEvent) => {
    e.preventDefault();
    const err = validate();
    if (err) {
      setStatusMessage(err);
      setStatusType("failure");
      return;
    }

    setIsSending(true);
    setProgress(10);
    setStatusMessage("Đang kết nối...");
    setStatusType("info");

    const timeoutPromise = new Promise<never>((_, reject) =>
      setTimeout(
        () =>
          reject(
            new Error(`Kết nối quá thời gian (${CONNECT_TIMEOUT_MS / 1000}s)`),
          ),
        CONNECT_TIMEOUT_MS,
      ),
    );

    try {
      // Bước 1: kết nối BLE
      await Promise.race([
        invoke("connect_robot_ble", { deviceId: selectedDevice }),
        timeoutPromise,
      ]);
      setConnected(true);
      setProgress(50);
      setStatusMessage("Đã kết nối, đang gửi thông tin WiFi qua BluFi...");

      // Bước 2: gửi WiFi
      await invoke("send_wifi_credentials", { ssid, password });
      setProgress(100);
      setStatusMessage(
        `Đã gửi WiFi "${ssid}" tới robot. Robot sẽ tự kết nối WiFi.`,
      );
      setStatusType("success");
      setPassword("");
    } catch (error) {
      setStatusMessage(`Lỗi: ${error}`);
      setStatusType("failure");
      setProgress(0);
      // Cố ngắt kết nối BLE nếu đã kết nối nhưng gửi WiFi thất bại
      if (connected) {
        try {
          await invoke("disconnect_robot_ble");
        } catch {
          /* ignore */
        }
        setConnected(false);
      }
    } finally {
      setIsSending(false);
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke("disconnect_robot_ble");
      setConnected(false);
      setProgress(0);
      setStatusMessage("Đã ngắt kết nối.");
      setStatusType("info");
    } catch (error) {
      setStatusMessage(`Lỗi ngắt kết nối: ${error}`);
      setStatusType("failure");
    }
  };

  return (
    <div className="min-h-screen w-full from-slate-50 to-blue-50 flex flex-col items-center justify-center p-6">
      {/* Header */}
      <div className="w-full max-w-2xl mb-6 text-center">
        <div className="flex items-center justify-center gap-2 mb-2">
          <HiOutlineWifi className="h-8 w-8 text-blue-600" />
          <h1 className="text-3xl font-extrabold text-gray-900">
            Kết nối Robot
          </h1>
        </div>
      </div>

      {/* Card chính */}
      <Card className="w-full max-w-2xl shadow-lg">
        <form onSubmit={handleConnectAndSend} className="flex flex-col gap-6">
          {/* ===== Bước 1: Thiết bị BLE ===== */}
          <section className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Badge color="info" size="sm">
                1
              </Badge>
              <h2 className="text-lg font-semibold text-gray-800">
                Chọn thiết bị là robot
              </h2>
            </div>

            <div>
              <Label htmlFor="ble-device"> Thiết bị </Label>
              <div className="flex gap-2 mt-1">
                <Select
                  id="ble-device"
                  value={selectedDevice}
                  onChange={(e) => setSelectedDevice(e.target.value)}
                  disabled={devices.length === 0 || connected || isSending}
                  className="flex-1 w-min-max"
                >
                  {devices.length === 0 ? (
                    <option value="">Không có thiết bị nào, hãy ấn quét</option>
                  ) : (
                    devices.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name ?? d.id}
                      </option>
                    ))
                  )}
                </Select>
                <Button
                  type="button"
                  color="light"
                  onClick={handleScan}
                  disabled={isScanning || connected || isSending}
                >
                  {isScanning ? (
                    <>
                      <Spinner size="sm" className="mr-2" />
                      Đang quét
                    </>
                  ) : (
                    <>
                      <HiOutlineRefresh className="mr-2 h-4 w-4" />
                      Quét
                    </>
                  )}
                </Button>
              </div>
            </div>
          </section>

          <HR />

          {/* ===== Bước 2: WiFi ===== */}
          <section className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Badge color="info" size="sm">
                2
              </Badge>
              <h2 className="text-lg font-semibold text-gray-800">
                Thông tin WiFi
              </h2>
            </div>

            <div>
              <Label htmlFor="wifi-ssid">Tên WiFi (SSID)</Label>
              <TextInput
                id="wifi-ssid"
                placeholder="Ví dụ: phong_steam"
                value={ssid}
                onChange={(e) => setSsid(e.target.value)}
                disabled={isSending}
                required
                icon={HiOutlineWifi}
              />
            </div>

            <div>
              <Label htmlFor="wifi-password">Mật khẩu WiFi</Label>
              <div className="relative mt-1">
                <button
                  type="button"
                  onClick={() => setShowPassword(!showPassword)}
                  disabled={isSending}
                  className="absolute inset-y-0 left-0 flex items-center pl-3 z-10 text-gray-500 hover:text-gray-700 focus:outline-none"
                >
                  {showPassword ? (
                    <HiOutlineEyeOff className="h-5 w-5" />
                  ) : (
                    <HiOutlineEye className="h-5 w-5" />
                  )}
                </button>
                <TextInput
                  id="wifi-password"
                  type={showPassword ? "text" : "password"}
                  placeholder="●●●●●●●●"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  disabled={isSending}
                  className="[&_input]:pl-10"
                />
              </div>
            </div>
          </section>

          <HR />

          {/* ===== Nút hành động ===== */}
          <div className="flex gap-3">
            <Button
              type="submit"
              className="flex-1"
              disabled={isSending || isScanning}
              color="blue"
            >
              {isSending ? (
                <>
                  <Spinner size="sm" className="mr-2" />
                  Đang xử lý...
                </>
              ) : (
                "Kết nối"
              )}
            </Button>

            {connected && (
              <Button
                type="button"
                color="failure"
                onClick={handleDisconnect}
                disabled={isSending}
              >
                Ngắt kết nối
              </Button>
            )}
          </div>

          {/* Thanh tiến trình */}
          {(isSending || progress > 0) && (
            <div>
              <Progress
                progress={progress}
                color={progress === 100 ? "green" : "blue"}
                size="sm"
              />
            </div>
          )}

          {/* Thông báo trạng thái */}
          {statusMessage && (
            <Alert
              color={
                statusType === "success"
                  ? "success"
                  : statusType === "failure"
                    ? "failure"
                    : "info"
              }
              icon={
                statusType === "success"
                  ? HiOutlineCheckCircle
                  : statusType === "failure"
                    ? HiOutlineExclamationCircle
                    : undefined
              }
            >
              <span className="text-sm break-words">{statusMessage}</span>
            </Alert>
          )}
        </form>
      </Card>
    </div>
  );
}
