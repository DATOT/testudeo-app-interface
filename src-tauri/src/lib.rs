mod ble;

use ble::types::BleState;
use btleplug::api::Manager as _;
use btleplug::platform::Manager;
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let manager = Manager::new().await.unwrap();
        let adapters = manager.adapters().await.unwrap();
        let adapter = adapters
            .into_iter()
            .next()
            .expect("Không tìm thấy Bluetooth Adapter nào!");

        tauri::Builder::default()
            .manage(BleState {
                adapter,
                connected_device: Arc::new(Mutex::new(None)),
            })
            .invoke_handler(tauri::generate_handler![
                ble::commands::get_ble_devices,
                ble::commands::scan_ble_devices,
                ble::commands::connect_robot_ble,
                ble::commands::send_wifi_credentials,
                ble::commands::disconnect_robot_ble,
            ])
            .run(tauri::generate_context!())
            .expect("Lỗi khi chạy ứng dụng Tauri");
    });
}