use btleplug::platform::{Adapter, Peripheral as PeripheralType};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BleDevice {
    pub id: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
}

pub struct BleState {
    pub adapter: Adapter,
    pub connected_device: Arc<Mutex<Option<PeripheralType>>>,
}