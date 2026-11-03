use crate::ble::types::{ BleDevice, BleState };
use crate::ble::utils::{ collect_devices, normalize_id, friendly_name };
use btleplug::api::{ Central, Peripheral, ScanFilter };
use std::time::Duration;
use tauri::State;
// Get cached/known devices or empty list initially
#[tauri::command]
pub async fn get_ble_devices(state: State<'_, BleState>) -> Result<Vec<BleDevice>, String> {
	collect_devices(&state.adapter).await
}

// Scan for BLE devices with a timeout
#[tauri::command]
pub async fn scan_ble_devices(
	state: State<'_, BleState>,
	timeout_ms: u64
) -> Result<Vec<BleDevice>, String> {
	// Stop any previous scan so we don't get stale results
	let _ = state.adapter.stop_scan().await;

	state.adapter
		.start_scan(ScanFilter::default()).await
		.map_err(|e| format!("Không bắt đầu quét được: {e}"))?;

	tokio::time::sleep(Duration::from_millis(timeout_ms)).await;

	state.adapter.stop_scan().await.map_err(|e| format!("Không dừng quét được: {e}"))?;

	collect_devices(&state.adapter).await
}

// Connect to the robot via BLE
#[tauri::command]
pub async fn connect_robot_ble(
	state: State<'_, BleState>,
	device_id: String
) -> Result<(), String> {
	let peripherals = state.adapter
		.peripherals().await
		.map_err(|e| format!("Không lấy được danh sách thiết bị: {e}"))?;

	// Compare normalized IDs so casing/braces don't matter
	let target_id = device_id.trim().to_uppercase();
	let target_peripheral = peripherals
		.into_iter()
		.find(|p| normalize_id(p) == target_id)
		.ok_or_else(|| {
			"Không tìm thấy thiết bị với ID này. Hãy quét lại rồi thử lại.".to_string()
		})?;

	target_peripheral.connect().await.map_err(|e| format!("Kết nối BLE thất bại: {e}"))?;

	// Nếu discover_services lỗi thì ngắt kết nối để không bị treo state
	if let Err(e) = target_peripheral.discover_services().await {
		let _ = target_peripheral.disconnect().await;
		return Err(format!("Không khám phá được dịch vụ GATT: {e}"));
	}

	let mut connected = state.connected_device.lock().await;
	*connected = Some(target_peripheral);

	Ok(())
}

// Send WiFi credentials via BLE (BluFi / Characteristic Write)
#[tauri::command]
pub async fn send_wifi_credentials(
	state: State<'_, BleState>,
	ssid: String,
	password: String
) -> Result<(), String> {
	let connected_lock = state.connected_device.lock().await;
	let peripheral = connected_lock
		.as_ref()
		.ok_or_else(|| "Chưa kết nối thiết bị BLE nào".to_string())?;

	// TODO: Implement BluFi frame encoding + characteristic write here.
	// BluFi GATT layout (Espressif):
	//   Service 0xFFFF
	//     Write  → 0xFF01
	//     Notify ← 0xFF02
	// You need to encode control frames (WiFi config = 0x02) with
	// sequence byte + CRC16-XMODEM checksum, chunked into fragments.

	// Simulating packet transmission delay for now:
	tokio::time::sleep(Duration::from_secs(2)).await;

	println!("Gửi SSID: {}, Password: *** tới Robot", ssid);

	Ok(())
}

// Disconnect BLE
#[tauri::command]
pub async fn disconnect_robot_ble(state: State<'_, BleState>) -> Result<(), String> {
	let mut connected_lock = state.connected_device.lock().await;
	if let Some(peripheral) = connected_lock.take() {
		peripheral.disconnect().await.map_err(|e| format!("Ngắt kết nối thất bại: {e}"))?;
	}
	Ok(())
}
