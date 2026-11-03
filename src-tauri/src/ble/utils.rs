use btleplug::api::{Central, Peripheral, ScanFilter};
use btleplug::platform::{Adapter, Peripheral as PeripheralType};
use crate::ble::types::{BleDevice, BleState};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;
/// Normalize a PeripheralId to a readable string.
/// macOS  → UUID (uppercased)
/// Win/Linux → MAC-style "AA:BB:CC:DD:EE:FF"
pub fn normalize_id(p: &PeripheralType) -> String {
    p.id()
        .to_string()
        .trim_matches(|c| c == '{' || c == '}')
        .to_uppercase()
}

/// Build a friendly display name.
/// Priority: local_name → "(không tên) <short-id>"
pub fn friendly_name(p: &PeripheralType, props_name: Option<String>) -> Option<String> {
    if let Some(n) = props_name {
        let trimmed = n.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    // BluFi devices sometimes have no name — give a fallback so it's selectable
    let id = normalize_id(p);
    Some(format!("(không tên) {}", id))
}

pub async fn collect_devices(adapter: &Adapter) -> Result<Vec<BleDevice>, String> {
    let peripherals = adapter
        .peripherals()
        .await
        .map_err(|e| format!("Không đọc được danh sách thiết bị: {e}"))?;

    let mut devices = Vec::with_capacity(peripherals.len());
    for peripheral in peripherals {
        let props = match peripheral.properties().await {
            Ok(Some(props)) => props,
            _ => continue, // skip devices with no properties (not yet resolved)
        };

        devices.push(BleDevice {
            id: normalize_id(&peripheral),
            name: friendly_name(&peripheral, props.local_name),
            rssi: props.rssi,
        });
    }

    // Sort: có tên trước, RSSI mạnh trước (gần hơn)
    devices.sort_by(|a, b| {
        let a_named = a.name.as_deref().map(|n| !n.starts_with("(không tên)")).unwrap_or(false);
        let b_named = b.name.as_deref().map(|n| !n.starts_with("(không tên)")).unwrap_or(false);
        b_named
            .cmp(&a_named)
            .then(b.rssi.unwrap_or(-127).cmp(&a.rssi.unwrap_or(-127)))
    });

    Ok(devices)
}
