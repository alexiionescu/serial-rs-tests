#![allow(unused_imports)]

use std::{
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Condvar, Mutex, RwLock,
    },
    thread::{self, sleep},
    time::Duration,
};

use log::{debug, error, info, trace, warn};
use rand_distr::{Distribution, Normal};

use crate::{
    test_esp::{EspTester, MSG_TYPE_REQ_CONFIG, MSG_TYPE_RES_CONFIG},
    ConnectArgs,
};

// rx_fifo_full_threshold
const READ_BUF_SIZE: usize = 128;
// EOT (CTRL-D)
const AT_CMD: u8 = 0x04;
const AT_ESC: u8 = 0x1b;
const AT_ESC_MASK: u8 = 0x30;

// max message size to receive
// leave some extra space for AT-CMD characters
const MAX_BUFFER_SIZE: usize = 5 * READ_BUF_SIZE + 20;
const RESET_BUFFER_SIZE: usize = MAX_BUFFER_SIZE - READ_BUF_SIZE;

type UartVec = Vec<u8>;

#[inline]
fn pop_escaped(buf: &[u8], offset: &mut usize) -> Option<u8> {
    if buf.is_empty() {
        return None;
    }
    if buf[0] == AT_ESC {
        if buf.len() == 1 {
            None
        } else {
            *offset += 2;
            match buf[1] {
                AT_ESC => Some(AT_ESC),
                b => Some(b & !AT_ESC_MASK),
            }
        }
    } else {
        *offset += 1;
        Some(buf[0])
    }
}
#[inline]
pub(crate) fn pop_all_escaped(buf: &[u8]) -> Vec<u8> {
    let mut offset = 0;
    let mut out = Vec::with_capacity(buf.len());
    while let Some(b) = pop_escaped(&buf[offset..], &mut offset) {
        out.push(b);
    }
    out
}
trait PushEscape {
    fn push_escaped(&mut self, b: u8);
    #[allow(unused)]
    fn pop_escaped(&mut self) -> Option<u8>;
}

impl PushEscape for UartVec {
    fn push_escaped(&mut self, b: u8) {
        match b {
            AT_CMD => {
                self.push(AT_ESC);
                self.push(b | AT_ESC_MASK);
            }
            AT_ESC => {
                self.push(AT_ESC);
                self.push(b);
            }
            b => self.push(b),
        }
    }

    fn pop_escaped(&mut self) -> Option<u8> {
        let b = self.pop()?;
        if b == AT_ESC {
            match self.pop() {
                Some(b) if b == AT_ESC => Some(AT_ESC),
                Some(b) => Some(b & !AT_ESC_MASK),
                _ => None,
            }
        } else {
            Some(b)
        }
    }
}

struct WriteData {
    seq_no: AtomicU16,
    wbuf: UartVec,
}
pub fn test(
    connect_args: ConnectArgs,
    no_send: bool,
    load_send: bool,
    mut at_cmd: bool,
    send: Vec<String>,
    send_time: Vec<u64>,
    esp_test: bool,
) {
    let mut serial = serialport::new(connect_args.port, connect_args.baud)
        .open()
        .expect("Failed to open port");
    let write_data = Arc::new(RwLock::new(WriteData {
        seq_no: AtomicU16::new(0),
        wbuf: Vec::with_capacity(MAX_BUFFER_SIZE),
    }));
    if esp_test {
        at_cmd = true;
    }
    let answer_data = Arc::new(Mutex::new(UartVec::with_capacity(MAX_BUFFER_SIZE)));
    let esp_tester = Arc::new(Mutex::new(EspTester::default()));
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = Arc::clone(&pair);

    if !no_send {
        let wlock_data = write_data.clone();
        let alock_data = answer_data.clone();

        let normal = Normal::new(
            if load_send { 70.0 } else { 500.0 },
            if load_send { 40.0 } else { 100.0 },
        )
        .unwrap();
        let mut wserial = serial
            .try_clone()
            .expect("Failed to clone port for writing");
        let mut hex_sends_iter = send.into_iter().map(|s| hex::decode(s).unwrap()).cycle();
        let mut send_time_iter = send_time.into_iter().cycle();

        thread::spawn(move || {
            let (lock, cvar) = &*pair;

            let mut seq_no = 0;
            let mut total_sent: usize = 0;
            let mut total_sent_bytes: usize = 0;
            let mut total_nack: usize = 0;
            loop {
                if !load_send {
                    let started = lock.lock().unwrap();
                    cvar.wait_timeout(
                        started,
                        Duration::from_secs(send_time_iter.next().unwrap_or(60)),
                    )
                    .ok();
                }
                let mut wdata = wlock_data.write().unwrap();
                if !load_send && wdata.seq_no.load(Ordering::Relaxed) > 0 {
                    warn!("last send was NG");
                    total_nack += 1;
                }

                seq_no += 1;

                wdata.wbuf.clear();
                let b = (seq_no) as u8;
                wdata.wbuf.push_escaped(b);
                let b = (seq_no >> 8) as u8;
                wdata.wbuf.push_escaped(b);
                let mut adata = alock_data.lock().unwrap();
                if !adata.is_empty() {
                    for &b in adata.iter() {
                        wdata.wbuf.push(b);
                    }
                    adata.clear();
                } else if let Some(hex) = hex_sends_iter.next() {
                    hex.iter().for_each(|b| wdata.wbuf.push_escaped(*b));
                } else if esp_test {
                    continue;
                } else {
                    wdata.wbuf.push_escaped(0xFF); // dummy message type
                    let len = normal.sample(&mut rand::thread_rng()) as usize;
                    for i in 0..len {
                        wdata.wbuf.push_escaped(i as u8);
                    }
                }
                let mut csum: u16 = 0;
                for b in &wdata.wbuf {
                    csum += *b as u16;
                    csum &= 0xFF;
                }
                wdata.wbuf.push_escaped(csum as u8);
                wdata.seq_no.store(seq_no, Ordering::SeqCst);

                if !load_send {
                    if wdata.wbuf.len() < 50 {
                        debug!(
                            "send SEQ:{:04X} {} bytes CKSUM:{} {}",
                            seq_no,
                            wdata.wbuf.len(),
                            csum,
                            hex::encode(&wdata.wbuf),
                        );
                    } else {
                        debug!(
                            "send SEQ:{:04X} {} bytes CKSUM:{} {} ... {}",
                            seq_no,
                            wdata.wbuf.len(),
                            csum,
                            hex::encode(&wdata.wbuf[..25]),
                            hex::encode(&wdata.wbuf[(wdata.wbuf.len() - 25)..])
                        );
                    }
                    trace!("send txt\n{}", &wdata.wbuf.escape_ascii().to_string());
                    trace!("send bin\n{}", hex::encode(&wdata.wbuf));
                }
                wdata.wbuf.push(AT_CMD);
                total_sent_bytes += wdata.wbuf.len();

                wserial.write_all(&wdata.wbuf).ok();
                wserial.flush().ok();

                #[cfg(feature = "async")]
                if at_cmd {
                    sleep(Duration::from_millis(50));
                    wserial.write_all(&[AT_CMD]).ok();
                    wserial.flush().ok();
                    sleep(Duration::from_millis(200));
                    let mut repeat_at_cmd = 1;
                    while wserial.bytes_to_read().unwrap() == 0
                        && repeat_at_cmd < wdata.wbuf.len() / 100 + 3
                    {
                        repeat_at_cmd += 1;
                        sleep(Duration::from_millis(200));

                        wserial.write_all(&[AT_CMD]).ok();
                        wserial.flush().ok();
                    }
                    if !load_send {
                        debug!("sent at_cmd {repeat_at_cmd} bytes");
                    }
                }

                total_sent += 1;
                if (!load_send && seq_no % 16 == 0) || seq_no % 1024 == 0 {
                    info!(
                        "STATS: sent:{:05} nack:{:03} {:07}B ",
                        total_sent, total_nack, total_sent_bytes
                    );
                }
            }
        });
    }

    let mut start = 0;
    let mut offset = 0;
    let mut rbuf = vec![0; MAX_BUFFER_SIZE];
    let (lock, cvar) = &*pair2;
    loop {
        if let Ok(n) = serial.read(&mut rbuf[start..]) {
            trace!("received {n} bytes");
            trace!(
                "received txt\n{}",
                &rbuf[start..(start + n)]
                    .escape_ascii()
                    .to_string()
                    .replace("\\x04", " <EOT>\n")
                    .replace("\\r\\n", " <CR><LF>\n")
                    .replace("\\r", " <CR>\n")
                    .replace("\\n", " <LF>\n")
            );
            trace!("received bin\n{}", hex::encode(&rbuf[start..(start + n)]));
            for i in start..(start + n) {
                if rbuf[i] == AT_CMD {
                    if i > offset {
                        let recv_size = i - offset - 1;
                        if recv_size >= 3 {
                            let mut recv_end = i - 1;
                            let mut recv_csum = rbuf[recv_end] as u32;
                            let mut csum: u32 = 0;
                            for b in &rbuf[offset..recv_end] {
                                csum += *b as u32;
                            }
                            if rbuf[recv_end - 1] == AT_ESC && rbuf[recv_end - 2] != AT_ESC {
                                //un-escape checksum
                                recv_end -= 1;
                                csum -= AT_ESC as u32;
                                if recv_csum != AT_ESC as u32 {
                                    recv_csum &= !AT_ESC_MASK as u32;
                                }
                            }
                            if csum & 0xFF == recv_csum {
                                if rbuf[offset] == 0
                                    && rbuf[offset + 1] == 0
                                    && rbuf[offset + 2] == 0x7E
                                {
                                    //debug print
                                    println!(
                                        "{}",
                                        &rbuf[(offset + 3)..recv_end].escape_ascii().to_string()
                                    );
                                } else if !no_send {
                                    let wdata = write_data.read().unwrap();
                                    let mut seq_no: u16 =
                                        pop_escaped(&rbuf[offset..recv_end], &mut offset).unwrap()
                                            as u16;
                                    seq_no += (pop_escaped(&rbuf[offset..recv_end], &mut offset)
                                        .unwrap()
                                        as u16)
                                        << 8;
                                    let msg_type =
                                        pop_escaped(&rbuf[offset..recv_end], &mut offset).unwrap();
                                    let _hdr_part =
                                        pop_escaped(&rbuf[offset..recv_end], &mut offset).unwrap();

                                    if msg_type & 0x80 != 0
                                        && wdata
                                            .seq_no
                                            .compare_exchange(
                                                seq_no,
                                                0,
                                                Ordering::Acquire,
                                                Ordering::Relaxed,
                                            )
                                            .is_ok()
                                    {
                                        debug!(
                                            "recv-ack SEQ:{:04X} T:{:02x} {} bytes {}",
                                            seq_no,
                                            msg_type,
                                            recv_size,
                                            hex::encode(&rbuf[offset..recv_end]),
                                        );
                                        // trace!("recv-ack bin\n{:02X?}", &rbuf[offset..recv_end]);
                                        info!("recv ACK for {seq_no}");
                                    } else {
                                        if i - offset < 50 {
                                            debug!(
                                                "recv-new SEQ:{:04X} T:{:02x} {} bytes {}",
                                                seq_no,
                                                msg_type,
                                                recv_size,
                                                hex::encode(&rbuf[offset..recv_end]),
                                            );
                                        } else {
                                            debug!(
                                                "recv-new SEQ:{:04X} T:{:02x} {} bytes {} ... {}",
                                                u16::to_be(seq_no),
                                                msg_type,
                                                recv_size,
                                                hex::encode(&rbuf[offset..(offset + 25)]),
                                                hex::encode(&rbuf[(recv_end - 25)..recv_end]),
                                            );
                                        }
                                        // trace!("recv-new bin\n{:02X?}", &rbuf[offset..recv_end]);
                                        if msg_type == MSG_TYPE_REQ_CONFIG {
                                            info!("<test> recv Req Config");
                                            let mut adata = answer_data.lock().unwrap();
                                            if adata.is_empty() {
                                                {
                                                    adata.push_escaped(MSG_TYPE_RES_CONFIG);
                                                    adata.push_escaped(0xFF);
                                                    for _ in 0..12 {
                                                        adata.push_escaped(0x00);
                                                    }
                                                }
                                                let mut started = lock.lock().unwrap();
                                                *started = true;
                                                // We notify the condvar that the value has changed.
                                                info!("notify Req Config");
                                                cvar.notify_one();
                                            } else {
                                                warn!("Cannot send res Config because data queue not empty!");
                                            }
                                        } else if esp_test {
                                            let escaped_data =
                                                pop_all_escaped(&rbuf[offset..recv_end]);
                                            let mut esp = esp_tester.lock().unwrap();
                                            esp.trace_esp_data(msg_type, &escaped_data[..]);
                                        }
                                    }
                                } else if recv_size > 5 {
                                    debug!(
                                        "recv {} bytes {:02X?} ... {:02X?}",
                                        recv_size,
                                        &rbuf[offset..(offset + 5)],
                                        &rbuf[(recv_end - 5)..i]
                                    );
                                    // trace!("recv bin\n{:02X?}", &rbuf[offset..recv_end]);
                                }
                            }
                        }
                    }
                    offset = i + 1;
                }
            }
            start += n;
            if offset == start || start >= RESET_BUFFER_SIZE {
                start = 0;
                offset = 0;
            }
        } else {
            sleep(Duration::from_millis(500));
        }
    }
}

pub(crate) fn generate(length: usize) {
    let mut csum: u32 = 0;
    for i in 1..length {
        print!("{}", i % 10);
        csum += 0x30 + (i % 10) as u32;
        csum &= 0xff;
    }
    while !(csum as u8).is_ascii_alphanumeric() {
        print!("0");
        csum += 0x30;
        csum &= 0xff;
    }
    println!("{}", (csum as u8) as char);
}

pub(crate) fn generate_bin(length: usize, checksum: Option<u8>) {
    let mut wbuf = UartVec::with_capacity(length * 2);
    let seq_no = 1;
    let b = (seq_no >> 8) as u8;
    wbuf.push_escaped(b);
    let b = (seq_no) as u8;
    wbuf.push_escaped(b);
    wbuf.push_escaped(0xFF); // dummy message type
    for i in 0..length {
        wbuf.push_escaped(i as u8);
    }
    let mut csum: u32 = 0;
    for b in &wbuf {
        csum += *b as u32;
        csum &= 0xFF;
        debug!("{:02x} -> {:02x}", *b, csum);
    }
    if let Some(cs) = checksum {
        while csum != cs as u32 {
            let b = 1;
            wbuf.push_escaped(b);
            csum += b as u32;
            csum &= 0xFF;
            debug!("{:02x} -> {:02x}", b, csum);
        }
    }
    wbuf.push_escaped(csum as u8);
    wbuf.push(AT_CMD);
    println!("{}", hex::encode(wbuf));
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[allow(dead_code)]
    fn test_csum_serial() {
        let mut data = "
            1b1b007e00416867254e3ff0000000000000000000001b340018a0764ead1d30001b1b61040404
            bc024100ffffffffffff1b340000000002bbffffffffffff02bca0764ead1d301b1b04
            121b344100ffffffffffff1b34000000001b3411ffffffffffff1b3412a0764ead1d301b3404"
            .to_string();
        data.retain(|c| !c.is_whitespace());
        let rbuf = hex::decode(data).unwrap();
        let mut offset = 0;
        let mut start = 0;
        let n = rbuf.len();
        let mut msg = 0;

        for i in start..(start + n) {
            if rbuf[i] == AT_CMD {
                if i > offset {
                    let recv_size = i - offset - 1;
                    if recv_size >= 3 {
                        let mut recv_end = i - 1;
                        let mut recv_csum = rbuf[recv_end] as u32;
                        let mut csum: u32 = 0;
                        for b in &rbuf[offset..recv_end] {
                            csum += *b as u32;
                        }
                        if rbuf[recv_end - 1] == AT_ESC && rbuf[recv_end - 2] != AT_ESC {
                            //un-escape checksum
                            recv_end -= 1;
                            csum -= AT_ESC as u32;
                            if recv_csum != AT_ESC as u32 {
                                recv_csum &= !AT_ESC_MASK as u32;
                            }
                        }
                        if msg == 1 || msg == 2 {
                            assert_eq!(recv_end, i - 2);
                        } else {
                            assert_eq!(recv_end, i - 1);
                        }
                        assert_eq!(csum & 0xFF, recv_csum);
                        msg += 1;
                    }
                }
                offset = i + 1;
            }
        }
        start += n;
        if offset == start || start >= RESET_BUFFER_SIZE {
            start = 0;
            offset = 0;
        }
        assert_eq!(start, 0);
        assert_eq!(offset, 0);
    }
}
