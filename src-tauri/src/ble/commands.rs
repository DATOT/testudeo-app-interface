use crate::ble::types::{BleDevice, BleState};
use crate::ble::utils::{collect_devices, normalize_id};
use btleplug::api::{Central, Peripheral, ScanFilter, WriteType};
use btleplug::platform::Peripheral as PeripheralType;
use std::collections::HashMap;
use std::time::Duration;
use tauri::State;

// ESP-IDF WiFiProv protocomm endpoints. Endpoint descriptors (0x2901) are
// preferred because ESP firmware may use a custom 128-bit service UUID.
const PROV_SESSION: &str = "prov-session";
const PROV_CONFIG: &str = "prov-config";

#[tauri::command]
pub async fn get_ble_devices(state: State<'_, BleState>) -> Result<Vec<BleDevice>, String> {
    collect_devices(&state.adapter).await
}

#[tauri::command]
pub async fn scan_ble_devices(
    state: State<'_, BleState>,
    timeout_ms: u64,
) -> Result<Vec<BleDevice>, String> {
    let _ = state.adapter.stop_scan().await;
    state
        .adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(|e| format!("Không bắt đầu quét BLE được: {e}"))?;

    tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
    state
        .adapter
        .stop_scan()
        .await
        .map_err(|e| format!("Không dừng quét BLE được: {e}"))?;

    collect_devices(&state.adapter).await
}

#[tauri::command]
pub async fn connect_robot_ble(
    state: State<'_, BleState>,
    device_id: String,
) -> Result<(), String> {
    let peripherals = state
        .adapter
        .peripherals()
        .await
        .map_err(|e| format!("Không lấy được danh sách thiết bị BLE: {e}"))?;

    let target_id = device_id.trim().to_uppercase();
    let target = peripherals
        .into_iter()
        .find(|p| normalize_id(p) == target_id)
        .ok_or_else(|| "Không tìm thấy robot. Hãy quét lại thiết bị BLE.".to_string())?;

    target
        .connect()
        .await
        .map_err(|e| format!("Kết nối BLE thất bại: {e}"))?;
    if let Err(e) = target.discover_services().await {
        let _ = target.disconnect().await;
        return Err(format!("Không khám phá được dịch vụ GATT: {e}"));
    }

    let mut connected = state.connected_device.lock().await;
    if let Some(previous) = connected.replace(target) {
        let _ = previous.disconnect().await;
    }
    Ok(())
}

/// Provision Wi-Fi through Espressif Arduino WiFiProv using Security 0.
/// Security 0 intentionally has no provisioning-layer encryption; use only
/// on trusted networks during prototyping.
#[tauri::command]
pub async fn send_wifi_credentials(
    state: State<'_, BleState>,
    ssid: String,
    password: String,
) -> Result<(), String> {
    let ssid = ssid.trim();
    if ssid.is_empty() || ssid.as_bytes().len() > 32 {
        return Err("SSID phải dài từ 1 đến 32 byte.".to_string());
    }
    if password.as_bytes().len() > 64 {
        return Err("Mật khẩu Wi-Fi không được vượt quá 64 byte.".to_string());
    }

    let connected = state.connected_device.lock().await;
    let peripheral = connected
        .as_ref()
        .ok_or_else(|| "Chưa kết nối robot BLE.".to_string())?;

    let endpoints = find_provisioning_endpoints(peripheral).await?;
    let session = endpoints
        .get(PROV_SESSION)
        .ok_or_else(|| "Robot không có endpoint WiFiProv prov-session.".to_string())?;
    let config = endpoints
        .get(PROV_CONFIG)
        .ok_or_else(|| "Robot không có endpoint WiFiProv prov-config.".to_string())?;

    // Establish the Security 0 session, then set and apply the network config.
    let session_reply = exchange(peripheral, session, &security0_session_request()).await?;
    validate_security0_session(&session_reply)?;

    let set_reply = exchange(peripheral, config, &set_wifi_config_request(ssid.as_bytes(), password.as_bytes())).await?;
    validate_config_response(&set_reply, 13, "thiết lập Wi-Fi")?;

    let apply_reply = exchange(peripheral, config, &apply_wifi_config_request()).await?;
    validate_config_response(&apply_reply, 15, "áp dụng cấu hình Wi-Fi")?;

    Ok(())
}

#[tauri::command]
pub async fn disconnect_robot_ble(state: State<'_, BleState>) -> Result<(), String> {
    let mut connected = state.connected_device.lock().await;
    if let Some(peripheral) = connected.take() {
        peripheral
            .disconnect()
            .await
            .map_err(|e| format!("Ngắt kết nối BLE thất bại: {e}"))?;
    }
    Ok(())
}

async fn find_provisioning_endpoints(
    peripheral: &PeripheralType,
) -> Result<HashMap<String, btleplug::api::Characteristic>, String> {
    let mut endpoints = HashMap::new();
    for characteristic in peripheral.characteristics() {
        let mut endpoint_name = None;
        for descriptor in &characteristic.descriptors {
            if descriptor.uuid.to_string().eq_ignore_ascii_case("00002901-0000-1000-8000-00805f9b34fb") {
                if let Ok(value) = peripheral.read_descriptor(descriptor).await {
                    let name = String::from_utf8_lossy(&value)
                        .trim_end_matches('\0')
                        .trim()
                        .to_ascii_lowercase();
                    if name == PROV_SESSION || name == PROV_CONFIG {
                        endpoint_name = Some(name);
                        break;
                    }
                }
            }
        }

        // The standard ESP WiFiProv UUIDs end in FF51 and FF52. Fall back to
        // these only if user-description descriptors are unavailable.
        let uuid = characteristic.uuid.to_string().to_ascii_lowercase();
        if endpoint_name.is_none() {
            if uuid_has_short_id(&uuid, "ff51") {
                endpoint_name = Some(PROV_SESSION.to_string());
            } else if uuid_has_short_id(&uuid, "ff52") {
                endpoint_name = Some(PROV_CONFIG.to_string());
            }
        }
        if let Some(name) = endpoint_name {
            endpoints.insert(name, characteristic);
        }
    }
    Ok(endpoints)
}

fn uuid_has_short_id(uuid: &str, short_id: &str) -> bool {
    uuid.split('-').any(|part| {
        part == short_id || part == format!("0000{short_id}")
    })
}

async fn exchange(
    peripheral: &PeripheralType,
    characteristic: &btleplug::api::Characteristic,
    request: &[u8],
) -> Result<Vec<u8>, String> {
    peripheral
        .write(characteristic, request, WriteType::WithResponse)
        .await
        .map_err(|e| format!("WiFiProv: gửi request thất bại: {e}"))?;

    // Let the peripheral process the write before reading its response.
    tokio::time::sleep(Duration::from_millis(100)).await;
    peripheral
        .read(characteristic)
        .await
        .map_err(|e| format!("WiFiProv: đọc response thất bại: {e}"))
}

fn security0_session_request() -> Vec<u8> {
    // SessionData { sec0: Sec0Payload { sc: S0SessionCmd {} } }
    // Field 10 -> Sec0Payload; field 20 -> empty S0SessionCmd.
    vec![0x52, 0x03, 0xA2, 0x01, 0x00]
}

fn set_wifi_config_request(ssid: &[u8], password: &[u8]) -> Vec<u8> {
    // NetworkConfigPayload.msg = TypeCmdSetWifiConfig (2).
    // NetworkConfigPayload.cmd_set_wifi_config is field 12.
    let mut wifi = Vec::with_capacity(ssid.len() + password.len() + 8);
    put_bytes_field(1, ssid, &mut wifi);
    put_bytes_field(2, password, &mut wifi);

    let mut request = vec![0x08, 0x02];
    put_bytes_field(12, &wifi, &mut request);
    request
}

fn apply_wifi_config_request() -> Vec<u8> {
    // NetworkConfigPayload.msg = TypeCmdApplyWifiConfig (4),
    // cmd_apply_wifi_config is empty field 14.
    vec![0x08, 0x04, 0x72, 0x00]
}

fn put_bytes_field(field: u32, value: &[u8], output: &mut Vec<u8>) {
    put_varint(((field << 3) | 2) as u64, output);
    put_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}

fn put_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn validate_security0_session(response: &[u8]) -> Result<(), String> {
    let mut cursor = 0;
    let mut has_sec0_payload = false;
    while cursor < response.len() {
        let (field, wire) = read_key(response, &mut cursor)?;
        match (field, wire) {
            (2, 0) => {
                let version = read_varint(response, &mut cursor)?;
                if version != 0 {
                    return Err(format!("Robot WiFiProv security version {version} is not Security 0."));
                }
            }
            (10, 2) => {
                let payload = read_bytes(response, &mut cursor)?;
                has_sec0_payload = payload.iter().any(|byte| *byte != 0) || !payload.is_empty();
            }
            _ => skip_field(response, &mut cursor, wire)?,
        }
    }
    if has_sec0_payload {
        Ok(())
    } else {
        Err("Robot WiFiProv session response was empty or invalid.".to_string())
    }
}

fn validate_config_response(response: &[u8], expected_field: u32, action: &str) -> Result<(), String> {
    let mut cursor = 0;
    while cursor < response.len() {
        let (field, wire) = read_key(response, &mut cursor)?;
        if field == expected_field && wire == 2 {
            let payload = read_bytes(response, &mut cursor)?;
            let mut nested = 0;
            while nested < payload.len() {
                let (nested_field, nested_wire) = read_key(&payload, &mut nested)?;
                if nested_field == 1 && nested_wire == 0 {
                    let status = read_varint(&payload, &mut nested)?;
                    if status == 0 {
                        return Ok(());
                    }
                    return Err(format!("WiFiProv: robot từ chối {action} (status {status})."));
                }
                skip_field(&payload, &mut nested, nested_wire)?;
            }
            // In proto3, enum value 0 (Success) is omitted from the wire.
            return Ok(());
        }
        skip_field(response, &mut cursor, wire)?;
    }
    Err(format!("WiFiProv: response không hợp lệ khi {action}."))
}

fn read_key(input: &[u8], cursor: &mut usize) -> Result<(u32, u8), String> {
    let key = read_varint(input, cursor)?;
    if key == 0 {
        return Err("WiFiProv: protobuf có field key không hợp lệ.".to_string());
    }
    Ok(((key >> 3) as u32, (key & 0x07) as u8))
}

fn read_varint(input: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *input
            .get(*cursor)
            .ok_or_else(|| "WiFiProv: protobuf response bị cắt cụt.".to_string())?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return Err("WiFiProv: protobuf varint vượt giới hạn.".to_string());
        }
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("WiFiProv: protobuf varint không hợp lệ.".to_string())
}

fn read_bytes<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], String> {
    let length = read_varint(input, cursor)? as usize;
    let end = cursor
        .checked_add(length)
        .filter(|end| *end <= input.len())
        .ok_or_else(|| "WiFiProv: protobuf field length không hợp lệ.".to_string())?;
    let bytes = &input[*cursor..end];
    *cursor = end;
    Ok(bytes)
}

fn skip_field(input: &[u8], cursor: &mut usize, wire: u8) -> Result<(), String> {
    match wire {
        0 => {
            read_varint(input, cursor)?;
        }
        1 => {
            *cursor = cursor
                .checked_add(8)
                .filter(|end| *end <= input.len())
                .ok_or_else(|| "WiFiProv: protobuf field length không hợp lệ.".to_string())?;
        }
        2 => {
            read_bytes(input, cursor)?;
        }
        5 => {
            *cursor = cursor
                .checked_add(4)
                .filter(|end| *end <= input.len())
                .ok_or_else(|| "WiFiProv: protobuf field length không hợp lệ.".to_string())?;
        }
        _ => return Err("WiFiProv: protobuf wire type không hỗ trợ.".to_string()),
    }
    Ok(())
}
