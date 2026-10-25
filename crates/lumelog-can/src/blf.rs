//! Vector BLF (Binary Logging Format) source. Reads `LOGG` file headers,
//! top-level `LOG_CONTAINER` objects (stored or zlib-deflated) and the CAN /
//! CAN-FD objects inside them, emitting [`Frame`]s for the shared CanPanel and
//! export paths. Format reference: python-can `blf.py` (the official Vector PDF
//! is not public); layout constants are pinned by the tests at the bottom.
//!
//! Only small-endian files are supported. Unsupported object types are counted
//! as `skipped`; corrupt/truncated data stops the stream and surfaces through
//! [`BlfSource::take_error`] instead of panicking.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use flate2::read::ZlibDecoder;

use crate::error::CanError;
use crate::frame::{Frame, FrameFlags, SourceStats};
use crate::source::FrameSource;

/// Fixed part of the file header we need (magic, header size, start SYSTEMTIME).
const FILE_HEADER_MIN: usize = 72;
const FILE_HEADER_MAX: u32 = 1024 * 1024;
const OBJ_HEADER_BASE: usize = 16;
const LOG_CONTAINER_HEADER: usize = 16;
/// Max size of one top-level or container-inner object (corruption guard).
const MAX_OBJECT_SIZE: u64 = 64 * 1024 * 1024;
/// Max decompressed bytes for one container (decompression-bomb guard).
const MAX_CONTAINER_DECOMPRESSED: u64 = 256 * 1024 * 1024;

const LOG_CONTAINER: u32 = 10;
const CAN_MESSAGE: u32 = 1;
const CAN_MESSAGE2: u32 = 86;
const CAN_FD_MESSAGE: u32 = 100;
const CAN_FD_MESSAGE_64: u32 = 101;

const COMPRESSION_NONE: u16 = 0;
const COMPRESSION_ZLIB: u16 = 2;

const TIME_TEN_MICS: u32 = 0x0000_0001;
const TIME_ONE_NANS: u32 = 0x0000_0002;
const REMOTE_FLAG: u8 = 0x80;
const FD_EDL: u8 = 0x01;
const FD_BRS: u8 = 0x02;
const FD_ESI: u8 = 0x04;
const FD64_RTR: u32 = 0x0010;
const FD64_EDL: u32 = 0x1000;
const FD64_BRS: u32 = 0x2000;
const FD64_ESI: u32 = 0x4000;

/// CAN-FD payload length by DLC code (ISO 11898-1).
const FD_DLC2LEN: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

fn dlc2len(code: u8) -> u8 {
    if code <= 15 {
        FD_DLC2LEN[code as usize]
    } else {
        64
    }
}

pub struct BlfSource {
    file: BufReader<File>,
    /// Epoch seconds from the file header `SYSTEMTIME`; `None` when invalid.
    start_epoch: Option<f64>,
    /// Bytes of a container-inner object that continues in the next container.
    tail: Vec<u8>,
    pending: VecDeque<Frame>,
    stats: SourceStats,
    error: Option<CanError>,
    next_line: u64,
}

impl BlfSource {
    pub fn open(path: &Path) -> Result<Self, CanError> {
        let mut file = BufReader::new(File::open(path)?);
        let mut head = [0u8; FILE_HEADER_MIN];
        read_exact(&mut file, &mut head).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                CanError::SourceFormat("blf: file too short for a header".into())
            } else {
                e.into()
            }
        })?;
        if &head[0..4] != b"LOGG" {
            return Err(CanError::SourceFormat("blf: unexpected file signature".into()));
        }
        let header_size = u32::from_le_bytes(head[4..8].try_into().unwrap());
        if header_size < FILE_HEADER_MIN as u32 || header_size > FILE_HEADER_MAX {
            return Err(CanError::SourceFormat(format!(
                "blf: invalid header size {header_size}"
            )));
        }
        if header_size as usize > FILE_HEADER_MIN {
            let mut skip = vec![0u8; header_size as usize - FILE_HEADER_MIN];
            file.read_exact(&mut skip)?;
        }
        let start_epoch = systemtime_epoch(&head[40..56]);
        Ok(Self {
            file,
            start_epoch,
            tail: Vec::new(),
            pending: VecDeque::new(),
            stats: SourceStats::default(),
            error: None,
            next_line: 1,
        })
    }

    /// Fatal parse error (stream already stopped), if any.
    pub fn take_error(&mut self) -> Option<CanError> {
        self.error.take()
    }

    fn pull(&mut self) -> Result<Option<Frame>, CanError> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                return Ok(Some(frame));
            }
            let mut base = [0u8; OBJ_HEADER_BASE];
            if !read_object_base(&mut self.file, &mut base)? {
                if !self.tail.is_empty() {
                    return Err(CanError::SourceFormat(
                        "blf: truncated object at end of file".into(),
                    ));
                }
                return Ok(None);
            }
            if &base[0..4] != b"LOBJ" {
                return Err(CanError::SourceFormat("blf: bad object signature".into()));
            }
            let obj_size = u32::from_le_bytes(base[8..12].try_into().unwrap()) as u64;
            let obj_type = u32::from_le_bytes(base[12..16].try_into().unwrap());
            if obj_size < OBJ_HEADER_BASE as u64 {
                return Err(CanError::SourceFormat(format!(
                    "blf: invalid object size {obj_size}"
                )));
            }
            if obj_size > MAX_OBJECT_SIZE {
                return Err(CanError::SourceFormat(format!(
                    "blf: object too large ({obj_size} bytes)"
                )));
            }
            let mut payload = vec![0u8; obj_size as usize - OBJ_HEADER_BASE];
            self.file.read_exact(&mut payload)?;
            skip_padding(&mut self.file, obj_size)?;

            if obj_type != LOG_CONTAINER {
                self.stats.skipped += 1; // LIN/FlexRay/statistics/... unsupported
                continue;
            }
            if payload.len() < LOG_CONTAINER_HEADER {
                return Err(CanError::SourceFormat("blf: container header too small".into()));
            }
            let method = u16::from_le_bytes(payload[0..2].try_into().unwrap());
            let data = &payload[LOG_CONTAINER_HEADER..];
            let body = match method {
                COMPRESSION_NONE => data.to_vec(),
                COMPRESSION_ZLIB => {
                    let mut out = Vec::new();
                    ZlibDecoder::new(data)
                        .take(MAX_CONTAINER_DECOMPRESSED + 1)
                        .read_to_end(&mut out)?;
                    if out.len() as u64 > MAX_CONTAINER_DECOMPRESSED {
                        return Err(CanError::SourceFormat(
                            "blf: decompressed container too large".into(),
                        ));
                    }
                    out
                }
                _ => {
                    self.stats.skipped += 1; // unsupported compression
                    continue;
                }
            };
            self.parse_container(&body)?;
        }
    }

    /// Parse complete `LOBJ` objects from one (decompressed) container, keeping a
    /// trailing partial object in `tail` for the next container.
    fn parse_container(&mut self, data: &[u8]) -> Result<(), CanError> {
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(data);
        let mut pos = 0usize;
        loop {
            let search_end = (pos + 8).min(buf.len());
            let found = (pos..search_end).find(|&i| buf[i..].starts_with(b"LOBJ"));
            let Some(obj_start) = found else {
                if buf.len() - pos > 8 {
                    return Err(CanError::SourceFormat(
                        "blf: could not find next object".into(),
                    ));
                }
                self.tail = buf[pos..].to_vec();
                return Ok(());
            };
            if obj_start + OBJ_HEADER_BASE > buf.len() {
                self.tail = buf[obj_start..].to_vec();
                return Ok(());
            }
            let header_version = u16::from_le_bytes(
                buf[obj_start + 6..obj_start + 8].try_into().unwrap(),
            );
            let obj_size = u32::from_le_bytes(
                buf[obj_start + 8..obj_start + 12].try_into().unwrap(),
            ) as usize;
            let obj_type = u32::from_le_bytes(
                buf[obj_start + 12..obj_start + 16].try_into().unwrap(),
            );
            if obj_size < OBJ_HEADER_BASE {
                return Err(CanError::SourceFormat("blf: invalid inner object size".into()));
            }
            if obj_size as u64 > MAX_OBJECT_SIZE {
                return Err(CanError::SourceFormat("blf: inner object too large".into()));
            }
            if obj_start + obj_size > buf.len() {
                self.tail = buf[obj_start..].to_vec();
                return Ok(());
            }
            let object = &buf[obj_start..obj_start + obj_size];
            self.parse_inner_object(object, header_version, obj_type)?;
            pos = obj_start + obj_size;
        }
    }

    fn parse_inner_object(
        &mut self,
        object: &[u8],
        header_version: u16,
        obj_type: u32,
    ) -> Result<(), CanError> {
        let ext_len = match header_version {
            1 => 16usize,
            2 => 24usize,
            _ => {
                self.stats.skipped += 1;
                return Ok(());
            }
        };
        if object.len() < OBJ_HEADER_BASE + ext_len {
            return Err(CanError::SourceFormat("blf: object header too small".into()));
        }
        let flags = u32::from_le_bytes(object[16..20].try_into().unwrap());
        let rel_ts = u64::from_le_bytes(object[24..32].try_into().unwrap());
        let factor = if flags & TIME_TEN_MICS != 0 && flags & TIME_ONE_NANS == 0 {
            1e-5
        } else {
            1e-9
        };
        let timestamp = Some(self.start_epoch.unwrap_or(0.0) + rel_ts as f64 * factor);
        let payload = &object[OBJ_HEADER_BASE + ext_len..];

        let frame = match obj_type {
            CAN_MESSAGE | CAN_MESSAGE2 => self.classic_frame(payload, timestamp)?,
            CAN_FD_MESSAGE => self.fd_frame(payload, timestamp)?,
            CAN_FD_MESSAGE_64 => self.fd64_frame(payload, timestamp)?,
            _ => {
                self.stats.skipped += 1;
                return Ok(());
            }
        };
        let mut frame = frame;
        frame.line_number = self.next_line;
        self.next_line += 1;
        self.stats.decoded += 1;
        self.pending.push_back(frame);
        Ok(())
    }

    fn classic_frame(
        &self,
        payload: &[u8],
        timestamp: Option<f64>,
    ) -> Result<Frame, CanError> {
        if payload.len() < 16 {
            return Err(CanError::SourceFormat("blf: CAN message too small".into()));
        }
        let channel = u16::from_le_bytes(payload[0..2].try_into().unwrap());
        let flags = payload[2];
        let dlc = payload[3];
        let can_id = u32::from_le_bytes(payload[4..8].try_into().unwrap());
        let mut data = [0u8; 64];
        let n = (dlc as usize).min(8);
        data[..n].copy_from_slice(&payload[8..8 + n]);
        Ok(Frame {
            line_number: 0, // filled by the caller
            timestamp,
            channel: channel.saturating_sub(1) as u8,
            can_id: can_id & 0x1FFF_FFFF,
            dlc,
            data,
            flags: FrameFlags {
                is_fd: false,
                brs: false,
                esi: false,
                is_remote: flags & REMOTE_FLAG != 0,
                is_error_frame: false,
            },
        })
    }

    fn fd_frame(&self, payload: &[u8], timestamp: Option<f64>) -> Result<Frame, CanError> {
        if payload.len() < 84 {
            return Err(CanError::SourceFormat("blf: CAN FD message too small".into()));
        }
        let channel = u16::from_le_bytes(payload[0..2].try_into().unwrap());
        let flags = payload[2];
        let dlc_code = payload[3];
        let can_id = u32::from_le_bytes(payload[4..8].try_into().unwrap());
        let fd_flags = payload[13];
        let valid = payload[14] as usize;
        let mut data = [0u8; 64];
        let n = valid.min(64);
        data[..n].copy_from_slice(&payload[20..20 + n]);
        Ok(Frame {
            line_number: 0,
            timestamp,
            channel: channel.saturating_sub(1) as u8,
            can_id: can_id & 0x1FFF_FFFF,
            dlc: dlc2len(dlc_code),
            data,
            flags: FrameFlags {
                is_fd: fd_flags & FD_EDL != 0,
                brs: fd_flags & FD_BRS != 0,
                esi: fd_flags & FD_ESI != 0,
                is_remote: flags & REMOTE_FLAG != 0,
                is_error_frame: false,
            },
        })
    }

    fn fd64_frame(&self, payload: &[u8], timestamp: Option<f64>) -> Result<Frame, CanError> {
        if payload.len() < 40 {
            return Err(CanError::SourceFormat(
                "blf: CAN FD 64 message too small".into(),
            ));
        }
        let channel = payload[0];
        let dlc_code = payload[1];
        let valid = payload[2] as usize;
        let can_id = u32::from_le_bytes(payload[4..8].try_into().unwrap());
        let fd_flags = u32::from_le_bytes(payload[12..16].try_into().unwrap());
        let ext_data_offset = payload[35] as usize;
        // VBLCANFDMessage64 is 40 bytes starting right after the object header;
        // its data follows immediately (payload offset 40, not 40 + ext_len).
        let data_start = 40usize;
        let available = if ext_data_offset != 0 {
            // ext_data_offset is in object coords; payload starts 16 bytes in.
            ext_data_offset.saturating_sub(OBJ_HEADER_BASE + data_start)
        } else {
            payload.len().saturating_sub(data_start)
        };
        let n = valid.min(available).min(64);
        let mut data = [0u8; 64];
        if data_start + n <= payload.len() {
            data[..n].copy_from_slice(&payload[data_start..data_start + n]);
        }
        Ok(Frame {
            line_number: 0,
            timestamp,
            channel: channel.saturating_sub(1),
            can_id: can_id & 0x1FFF_FFFF,
            dlc: dlc2len(dlc_code),
            data,
            flags: FrameFlags {
                is_fd: fd_flags & FD64_EDL != 0,
                brs: fd_flags & FD64_BRS != 0,
                esi: fd_flags & FD64_ESI != 0,
                is_remote: fd_flags & FD64_RTR != 0,
                is_error_frame: false,
            },
        })
    }
}

impl FrameSource for BlfSource {
    fn next_frame(&mut self) -> Option<Frame> {
        if self.error.is_some() {
            return None;
        }
        match self.pull() {
            Ok(frame) => frame,
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    fn stats(&self) -> SourceStats {
        self.stats
    }
}

/// Read exactly `buf.len()` bytes or report clean EOF before any byte.
fn read_object_base(file: &mut BufReader<File>, buf: &mut [u8; OBJ_HEADER_BASE]) -> Result<bool, CanError> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => {
                return Err(CanError::SourceFormat(
                    "blf: truncated object header".into(),
                ))
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}

fn read_exact(file: &mut BufReader<File>, buf: &mut [u8]) -> std::io::Result<()> {
    file.read_exact(buf)
}

fn skip_padding(file: &mut BufReader<File>, obj_size: u64) -> Result<(), CanError> {
    let pad = (obj_size % 4) as usize;
    if pad != 0 {
        let mut buf = [0u8; 4];
        file.read_exact(&mut buf[..pad])?;
    }
    Ok(())
}

/// Convert a BLF `SYSTEMTIME` (16 bytes, eight little-endian u16 fields:
/// year, month, day-of-week, day, hour, minute, second, millisecond) to epoch
/// seconds. All-zero or an invalid date yields `None`.
fn systemtime_epoch(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 16 {
        return None;
    }
    let field = |i: usize| u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]) as i64;
    let (year, month, day) = (field(0), field(1), field(3));
    let (hour, minute, second, ms) = (field(4), field(5), field(6), field(7));
    if year == 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 || ms > 999 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days as f64 * 86_400.0 + (hour * 3600 + minute * 60 + second) as f64 + ms as f64 / 1000.0)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::PathBuf;

    // ---- fixture writer (mirrors the python-can object layout) --------------

    fn push_u16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn push_u32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    fn object(obj_type: u32, payload: &[u8], rel_ns: u64, flags: u32) -> Vec<u8> {
        let obj_size = (OBJ_HEADER_BASE + 16 + payload.len()) as u32;
        let mut v = Vec::new();
        v.extend_from_slice(b"LOBJ");
        push_u16(&mut v, (OBJ_HEADER_BASE + 16) as u16);
        push_u16(&mut v, 1);
        push_u32(&mut v, obj_size);
        push_u32(&mut v, obj_type);
        push_u32(&mut v, flags);
        push_u16(&mut v, 0); // client index
        push_u16(&mut v, 0); // object version
        v.extend_from_slice(&rel_ns.to_le_bytes());
        v.extend_from_slice(payload);
        let pad = obj_size as usize % 4;
        v.extend_from_slice(&vec![0u8; pad]);
        v
    }

    fn container(inner: &[u8], compress: bool) -> Vec<u8> {
        let (method, data) = if compress {
            let mut enc = flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::default(),
            );
            enc.write_all(inner).unwrap();
            (COMPRESSION_ZLIB, enc.finish().unwrap())
        } else {
            (COMPRESSION_NONE, inner.to_vec())
        };
        let mut header = Vec::new();
        push_u16(&mut header, method);
        header.extend_from_slice(&[0u8; 6]);
        push_u32(&mut header, inner.len() as u32);
        header.extend_from_slice(&[0u8; 4]);
        let payload_len = LOG_CONTAINER_HEADER + data.len();
        let obj_size = (OBJ_HEADER_BASE + payload_len) as u32;
        let mut v = Vec::new();
        v.extend_from_slice(b"LOBJ");
        push_u16(&mut v, OBJ_HEADER_BASE as u16);
        push_u16(&mut v, 1);
        push_u32(&mut v, obj_size);
        push_u32(&mut v, LOG_CONTAINER);
        v.extend_from_slice(&header);
        v.extend_from_slice(&data);
        let pad = obj_size as usize % 4;
        v.extend_from_slice(&vec![0u8; pad]);
        v
    }

    fn start_time() -> [u16; 8] {
        [2020, 1, 4, 2, 3, 4, 5, 6] // 2020-01-02T03:04:05.006Z
    }

    fn file(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"LOGG");
        push_u32(&mut v, 144);
        v.resize(40, 0);
        for x in start_time() {
            push_u16(&mut v, x);
        }
        v.resize(144, 0);
        for o in objects {
            v.extend_from_slice(o);
        }
        v
    }

    fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lumelog-blf-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.blf");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn drain(src: &mut BlfSource) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(f) = src.next_frame() {
            out.push(f);
        }
        out
    }

    fn classic_payload(channel: u16, flags: u8, dlc: u8, id: u32, data: &[u8]) -> Vec<u8> {
        let mut p = Vec::new();
        push_u16(&mut p, channel);
        p.push(flags);
        p.push(dlc);
        push_u32(&mut p, id);
        let mut d = [0u8; 8];
        d[..data.len().min(8)].copy_from_slice(&data[..data.len().min(8)]);
        p.extend_from_slice(&d);
        p
    }

    fn fd100_payload(dlc_code: u8, id: u32, valid: u8, fd_flags: u8, data: &[u8]) -> Vec<u8> {
        let mut p = Vec::new();
        push_u16(&mut p, 2); // channel 2 -> 1
        p.push(0);
        p.push(dlc_code);
        push_u32(&mut p, id);
        push_u32(&mut p, 0);
        p.push(0);
        p.push(fd_flags);
        p.push(valid);
        p.extend_from_slice(&[0u8; 5]);
        let mut d = [0u8; 64];
        d[..data.len().min(64)].copy_from_slice(&data[..data.len().min(64)]);
        p.extend_from_slice(&d);
        p
    }

    fn fd64_payload(dlc_code: u8, id: u32, valid: u8, fd_flags: u32, data: &[u8]) -> Vec<u8> {
        let mut p = vec![3u8, dlc_code, valid, 0];
        push_u32(&mut p, id);
        push_u32(&mut p, 0);
        push_u32(&mut p, fd_flags);
        p.extend_from_slice(&[0u8; 18]);
        p.push(0); // Rx
        p.push(0); // no extended data offset
        p.extend_from_slice(&[0u8; 4]);
        let mut d = [0u8; 64];
        d[..data.len().min(64)].copy_from_slice(&data[..data.len().min(64)]);
        p.extend_from_slice(&d);
        p
    }

    // ---- tests --------------------------------------------------------------

    #[test]
    fn classic_frame_fields_and_time() {
        let obj = object(
            CAN_MESSAGE,
            &classic_payload(2, 0x81, 3, 0x8000_0123, &[0xAA, 0xBB, 0xCC]),
            1_500_000_000,
            TIME_ONE_NANS,
        );
        let path = write_temp("classic", &file(&[container(&obj, false)]));
        let mut src = BlfSource::open(&path).unwrap();
        let frames = drain(&mut src);
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!(f.channel, 1);
        assert_eq!(f.can_id, 0x123);
        assert!(f.flags.is_remote); // 0x80
        assert_eq!(f.dlc, 3);
        assert_eq!(&f.data[..3], &[0xAA, 0xBB, 0xCC]);
        assert_eq!(f.line_number, 1);
        let ts = f.timestamp.unwrap();
        assert!((ts - 1_577_934_246.506).abs() < 1e-6, "ts={ts}");
        assert!(src.take_error().is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn fd100_maps_lengths_and_flags() {
        let obj = object(
            CAN_FD_MESSAGE,
            &fd100_payload(9, 0x456, 12, FD_EDL | FD_BRS | FD_ESI, &[1; 12]),
            0,
            TIME_ONE_NANS,
        );
        let path = write_temp("fd100", &file(&[container(&obj, false)]));
        let mut src = BlfSource::open(&path).unwrap();
        let f = src.next_frame().unwrap();
        assert_eq!(f.channel, 1);
        assert_eq!(f.can_id, 0x456);
        assert_eq!(f.dlc, 12);
        assert!(f.flags.is_fd && f.flags.brs && f.flags.esi);
        assert_eq!(&f.data[..12], &[1u8; 12]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn fd64_maps_64_bytes_and_direction() {
        let mut data = [0u8; 64];
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }
        let obj = object(
            CAN_FD_MESSAGE_64,
            &fd64_payload(15, 0x789, 64, FD64_EDL | FD64_BRS, &data),
            0,
            TIME_ONE_NANS,
        );
        let path = write_temp("fd64", &file(&[container(&obj, false)]));
        let mut src = BlfSource::open(&path).unwrap();
        let f = src.next_frame().unwrap();
        assert_eq!(f.channel, 2);
        assert_eq!(f.dlc, 64);
        assert!(f.flags.is_fd && f.flags.brs && !f.flags.esi);
        assert_eq!(&f.data[..], &data[..]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn compressed_container_matches_uncompressed() {
        let obj = object(
            CAN_MESSAGE,
            &classic_payload(1, 0, 2, 0x10, &[0xDE, 0xAD]),
            0,
            TIME_ONE_NANS,
        );
        let plain = write_temp("plain", &file(&[container(&obj, false)]));
        let zipped = write_temp("zipped", &file(&[container(&obj, true)]));
        let mut a = BlfSource::open(&plain).unwrap();
        let mut b = BlfSource::open(&zipped).unwrap();
        assert_eq!(drain(&mut a), drain(&mut b));
        std::fs::remove_dir_all(plain.parent().unwrap()).ok();
        std::fs::remove_dir_all(zipped.parent().unwrap()).ok();
    }

    #[test]
    fn split_object_across_containers_emits_once() {
        let obj = object(
            CAN_MESSAGE,
            &classic_payload(1, 0, 8, 0x20, &[1, 2, 3, 4, 5, 6, 7, 8]),
            0,
            TIME_ONE_NANS,
        );
        let mid = obj.len() / 2;
        let mut v = file(&[]);
        v.extend_from_slice(&container(&obj[..mid], false));
        v.extend_from_slice(&container(&obj[mid..], false));
        let path = write_temp("split", &v);
        let mut src = BlfSource::open(&path).unwrap();
        let frames = drain(&mut src);
        assert_eq!(frames.len(), 1);
        assert_eq!(&frames[0].data[..4], &[1, 2, 3, 4]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn ten_microsecond_flag_scales() {
        let obj = object(
            CAN_MESSAGE,
            &classic_payload(1, 0, 0, 0x30, &[]),
            100,
            TIME_TEN_MICS,
        );
        let path = write_temp("tenus", &file(&[container(&obj, false)]));
        let mut src = BlfSource::open(&path).unwrap();
        let f = src.next_frame().unwrap();
        assert!((f.timestamp.unwrap() - 1_577_934_245.007).abs() < 1e-6);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn unknown_objects_are_skipped() {
        let unknown = object(999, b"whatever", 0, TIME_ONE_NANS);
        let msg = object(
            CAN_MESSAGE,
            &classic_payload(1, 0, 1, 0x40, &[7]),
            0,
            TIME_ONE_NANS,
        );
        let inner = [unknown, msg].concat();
        let path = write_temp("unknown", &file(&[container(&inner, false)]));
        let mut src = BlfSource::open(&path).unwrap();
        let frames = drain(&mut src);
        assert_eq!(frames.len(), 1);
        assert_eq!(src.stats().decoded, 1);
        assert_eq!(src.stats().skipped, 1);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn bad_magic_is_error() {
        let mut v = file(&[]);
        v[0] = b'X';
        let path = write_temp("badmagic", &v);
        let err = match BlfSource::open(&path) {
            Err(e) => e,
            Ok(_) => panic!("bad magic must be rejected"),
        };
        assert_eq!(err.ipc_code(), "source_format");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn truncated_container_is_error() {
        let obj = object(
            CAN_MESSAGE,
            &classic_payload(1, 0, 1, 0x50, &[1]),
            0,
            TIME_ONE_NANS,
        );
        let mut v = file(&[container(&obj, false)]);
        v.truncate(v.len() - 3);
        let path = write_temp("truncated", &v);
        let mut src = BlfSource::open(&path).unwrap();
        assert!(src.next_frame().is_none());
        assert!(src.take_error().is_some());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn frame_line_numbers_increase() {
        let objs = [
            object(CAN_MESSAGE, &classic_payload(1, 0, 1, 0x60, &[1]), 0, TIME_ONE_NANS),
            object(CAN_MESSAGE, &classic_payload(1, 0, 1, 0x61, &[2]), 0, TIME_ONE_NANS),
        ]
        .concat();
        let path = write_temp("lines", &file(&[container(&objs, false)]));
        let mut src = BlfSource::open(&path).unwrap();
        let frames = drain(&mut src);
        assert_eq!(frames.iter().map(|f| f.line_number).collect::<Vec<_>>(), vec![1, 2]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
