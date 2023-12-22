use log::*;
use std::{collections::HashMap, fmt::Display, time::{Instant, Duration}};

// message header offsets and size
// const HDR_SEQ_LB: usize = 0;
// const HDR_SEQ_HB: usize = HDR_SEQ_LB + 1;
// const HDR_TYPE: usize = HDR_SEQ_HB + 1;
// const HDR_PART: usize = HDR_TYPE + 1;
// const DATA_HEADER_SIZE: usize = HDR_PART + 1;


const MSG_TYPE_RES: u8 = 0x20;
// request 0x00 - 0x1F
// responses 0x20 - 0x3f = (0x00 - 0x1F | MSG_TYPE_RES)
pub const MSG_TYPE_REQ_CONFIG: u8 = 0x00;
pub const MSG_TYPE_RES_CONFIG: u8 = MSG_TYPE_REQ_CONFIG | MSG_TYPE_RES;
// push info 0x40 - 0x5F = (MSG_TYPE_PUSH | 0x00 - 0x1F)
// push responses 0x60 - 0x7F = (MSG_TYPE_PUSH | MSG_TYPE_RES | 0x00 - 0x1F)
const MSG_TYPE_PUSH: u8 = 0x40;
const MSG_TYPE_PUSH_NETSTAT: u8 = MSG_TYPE_PUSH | 0x01;
const STAT_SIZE: usize = 13;
const MSG_TYPE_PUSH_GPIO: u8 = MSG_TYPE_PUSH | 0x02;

/// ntfy msg type, bcast only
pub const MSG_TYPE_NOTIFY: u8 = 0x7E;
// end mesage types
// const NOTIFY_MSG_LEN: usize = 1;

// begin notify types
// notifies 0x00 - 0x3F
// const NOTIFY_CONFIG_CHANGED: u8 = 0x01;
// const NOTIFY_PIN_LED: u8 = 0x02;
// const NOTIFY_RGB_LED: u8 = 0x03;
// const NOTIFY_NEIGH_QUERY: u8 = 0x04;
// const NOTIFY_NEIGH_UPDATE: u8 = 0x05;
// push notifies are 0x40 - 0x5F = (MSG_TYPE_PUSH | 0x00 - 0x1F)
// end notify types

const ESP_NAMES: phf::Map<&'static str,&'static str> = phf::phf_map! {
    "6867254e3ff0" => "First Repeater",
    "7cdfa1dee298" => "Led Repeater",
    "6867254d6258" => "R.Build Room",
    "6867254f88f8" => "R.ESP.Support",
    "6867254eed84" => "Red Repeater",
    "7cdfa1dee03c" => "Support Exit",
    "a0764ead1d30" => "COORDINATOR"
};

#[derive(Debug)]
struct Timestamp(Instant);
impl Default for Timestamp {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl From<Instant> for Timestamp {
    fn from(value: Instant) -> Self {
        Self(value)
    }
}

#[derive(Default, PartialEq, Eq, Hash, Clone)]
struct MacAddr([u8; 6]);

impl From<&[u8]> for MacAddr {
    fn from(value: &[u8]) -> Self {
        Self(std::array::from_fn(|i| value[value.len() - 6 + i]))
    }
}

impl AsRef<[u8]> for MacAddr {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_code = hex::encode(self);
        if let Some(name) = ESP_NAMES.get(&hex_code) {
            name.fmt(f)
        } else {
            hex_code.fmt(f)
        }
    }
}

impl std::fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MacAddr").field(&format!("{}",&self)).finish()
    }
}

#[derive(Default,Debug)]
struct EspDevice {
    addr: MacAddr,
    net_stat_ts: u16,
    last_push_id: u16,
    last_seen: Timestamp,
    last_seen_gap: Duration,
    next_node: Option<MacAddr>,
    total_resent: u32,
    total_sent: u32,
    total_failed: u32,
    total_failed_queued: u32,
    rssi: u32,
    snr: u32,
    rssi_cnt: u32,
    total_rx_ntfy: u32,
    total_rx_bcast: u32,
    total_rx_direct: u32,
    total_relay_req: u32,
    total_relay_ntfy: u32,
}

impl EspDevice {
    fn new(addr: MacAddr) -> Self {
        Self {
            addr,
            ..Default::default()
        }
    }

    fn decode_netstat(&mut self, msg: &[u8]) {
        let next_node = MacAddr::from(msg);
        let is_coordinator = msg[1] == 0xFF;
        if !is_coordinator {
            if self.next_node.is_none() {
                self.next_node = Some(next_node);
            }
            else if &next_node != self.next_node.as_ref().unwrap() {
                self.next_node = Some(next_node);
                warn!("{:>14}>ESP Changed Next Node: {}", self.addr, self.next_node.as_ref().unwrap());
                self.rssi_cnt = 0;
                self.rssi = 0;
                self.snr = 0;
            }
        }

        let net_stat_ts = u16::from_be_bytes((&msg[11..STAT_SIZE]).try_into().unwrap());
        if net_stat_ts > self.net_stat_ts && self.net_stat_ts > 0 {
            let ts_gap = net_stat_ts - self.net_stat_ts - 1;
            if ts_gap > 3 {
                error!(
                    "{:>14}>ESP Net Stat Timestamp skipped:{} gap:{:?}",
                    self.addr,
                    ts_gap, 
                    self.last_seen_gap
                ); 
            } else if ts_gap > 0 {
                warn!(
                    "{:>14}>ESP Net Stat Timestamp skipped:{} gap:{:?}",
                    self.addr,
                    ts_gap, 
                    self.last_seen_gap
                ); 
            } 
        }
        self.net_stat_ts = net_stat_ts;

        self.total_rx_ntfy += msg[6] as u32;
        self.total_rx_bcast += msg[7] as u32;
        self.total_rx_direct += msg[8] as u32;
        self.total_relay_req += msg[9] as u32;
        self.total_relay_ntfy += msg[10] as u32;
        if is_coordinator {
            info!(
                "{:>14}>ESP Net Stat TS:{:04x} PUSH:{:04x} NFY:{:6}/{:<6} RXB:{:5} RXD:{:5}",
                self.addr, self.net_stat_ts, 
                self.last_push_id,
                    self.total_rx_ntfy, self.total_relay_ntfy,
                self.total_rx_bcast, self.total_rx_direct,
            );
        } else {
            if msg[0] > 0 {
                self.rssi_cnt += 1;
                self.rssi += msg[0] as u32;
                self.snr += msg[1] as u32;
            }
            self.total_resent += msg[2] as u32;
            self.total_failed_queued += msg[3] as u32;
            self.total_failed += msg[4] as u32;
            self.total_sent += msg[5] as u32;

            if msg[3] > 0 || msg[4] > 0 {
                warn!(
                    "{:>14}>ESP Net Stat TS:{:04X} PUSH:{:04x} NFY:{:6}/{:<6} RSSI:{:3} SNR:{:3} FAILQ:{:3} FAIL:{:3} SENT:{:5} RXB:{:5} RXD:{:5} RLY:{:5} -> {}",
                    self.addr, self.net_stat_ts, self.last_push_id,
                    self.total_rx_ntfy, self.total_relay_ntfy, 
                    self.rssi.checked_div(self.rssi_cnt).unwrap_or_default(), 
                    self.snr.checked_div(self.rssi_cnt).unwrap_or_default(),
                    self.total_failed_queued,self.total_failed,
                    self.total_sent,  self.total_rx_bcast, self.total_rx_direct,
                    self.total_relay_req, 
                    self.next_node.as_ref().unwrap()
                );
            } else {
                info!(
                    "{:>14}>ESP Net Stat TS:{:04X} PUSH:{:04x} NFY:{:6}/{:<6} RSSI:{:3} SNR:{:3} FAILQ:{:3} FAIL:{:3} SENT:{:5} RXB:{:5} RXD:{:5} RLY:{:5} -> {}",
                    self.addr, self.net_stat_ts, self.last_push_id,
                    self.total_rx_ntfy, self.total_relay_ntfy, 
                    self.rssi.checked_div(self.rssi_cnt).unwrap_or_default(), 
                    self.snr.checked_div(self.rssi_cnt).unwrap_or_default(),
                    self.total_failed_queued,self.total_failed,
                    self.total_sent,  self.total_rx_bcast, self.total_rx_direct,
                    self.total_relay_req, 
                    self.next_node.as_ref().unwrap()
                );
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct EspTester {
    esp_devices: HashMap<MacAddr, EspDevice>,
}

impl EspTester {
    pub fn trace_esp_data(&mut self, msg_type: u8, data: &[u8]) {
        match msg_type {
            MSG_TYPE_PUSH_NETSTAT => self.decode_push_netstat(data),
            MSG_TYPE_PUSH_GPIO => self.decode_push_gpio(data),
            MSG_TYPE_NOTIFY => self.decode_notify(data),
            _ => (),
        };
    }

    fn decode_push(&mut self, data: &[u8]) -> &mut EspDevice {
        let esp_device = self
            .esp_devices
            .entry(MacAddr::from(data))
            .or_insert(EspDevice::new(MacAddr::from(data)));
        esp_device.last_seen_gap = esp_device.last_seen.0.elapsed();
        esp_device.last_seen = Instant::now().into();
        esp_device.last_push_id = u16::from_be_bytes(
            (&data[(data.len() - 8)..(data.len() - 6)])
                .try_into()
                .unwrap(),
        );
        esp_device
    }

    fn decode_push_netstat(&mut self, data: &[u8]) {
        let esp_device = self.decode_push(data);
        esp_device.decode_netstat(&data[..(STAT_SIZE + 6)]);
    }

    fn decode_push_gpio(&mut self, data: &[u8]) {
        let _esp_device = self.decode_push(data);
    }

    fn decode_notify(&mut self, data: &[u8]) {
        let mac = MacAddr::from(&data[1..7]);
        if data[0] & MSG_TYPE_PUSH != 0 {
            let esp_device = self.esp_devices
                .entry(mac)
                .or_insert(EspDevice::new(MacAddr::from(&data[1..7])));
            esp_device.last_seen_gap = esp_device.last_seen.0.elapsed();
            esp_device.last_seen = Instant::now().into();
            let push_id = u16::from_be_bytes(
                (&data[(data.len() - 2)..])
                    .try_into()
                    .unwrap(),
            );
            if esp_device.last_push_id < push_id // newer
            || esp_device.last_push_id - push_id > 100 // or re-cycled push id
            {
                esp_device.last_push_id = push_id;
                match data[0] {
                    MSG_TYPE_PUSH_NETSTAT => esp_device.decode_netstat(&data[7..(7 + STAT_SIZE + 6)]),
                    MSG_TYPE_PUSH_GPIO => (),
                    _ => (),
                }
            }
        } else {
            warn!("{:>14}>ESP NFY{:02X} non-PUSH ", mac, data[0]);
        }
    }
}

#[cfg(test)]
mod test {
    use crate::test_serial::pop_all_escaped;
    use super::*;

    #[test]
    fn test_pop_allescaped() {
        let data = hex::decode("c92300000002010001010106416867254eed8406457cdfa1dee03c").unwrap();
        let escaped_data = pop_all_escaped(&data);
        let mut esp_tester = EspTester::default();
        esp_tester.trace_esp_data(MSG_TYPE_PUSH_NETSTAT, &escaped_data);
        println!("esp_devices: {:#?}",esp_tester.esp_devices);
    }

    #[test]
    fn test_notify_push() {
        let data = hex::decode("416867254e3ff0ed47000000000c0000000c0001a0764ead1d3000170b04").unwrap();
        let escaped_data = pop_all_escaped(&data);
        let mut esp_tester = EspTester::default();
        esp_tester.trace_esp_data(MSG_TYPE_NOTIFY, &escaped_data);
        let mac_addr = MacAddr::from(hex::decode("6867254e3ff0").unwrap().as_slice());
        assert!(esp_tester.esp_devices.contains_key(&mac_addr));
        assert_eq!(format!("{}",esp_tester.esp_devices[&mac_addr].next_node.as_ref().unwrap()),"COORDINATOR");
    }
}