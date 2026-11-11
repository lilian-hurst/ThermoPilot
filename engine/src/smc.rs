//! Accès bas niveau au SMC (System Management Controller) via IOKit.
//! Lecture sans privilèges, écriture (contrôle des ventilateurs) nécessite root.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CString};
use std::sync::Mutex;

type KernReturn = i32;
type IoObject = u32;
type IoConnect = u32;
type MachPort = u32;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: MachPort, matching: *mut c_void) -> IoObject;
    fn IOServiceOpen(service: IoObject, owning_task: MachPort, typ: u32, connect: *mut IoConnect) -> KernReturn;
    fn IOServiceClose(connect: IoConnect) -> KernReturn;
    fn IOObjectRelease(obj: IoObject) -> KernReturn;
    fn IOConnectCallStructMethod(
        connection: IoConnect,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> KernReturn;
}

extern "C" {
    static mach_task_self_: MachPort;
}

const KERNEL_INDEX_SMC: u32 = 2;
const CMD_READ_BYTES: u8 = 5;
const CMD_WRITE_BYTES: u8 = 6;
const CMD_READ_INDEX: u8 = 8;
const CMD_READ_KEYINFO: u8 = 9;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Vers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PLimit {
    version: u16,
    length: u16,
    cpu_p_limit: u32,
    gpu_p_limit: u32,
    mem_p_limit: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyData {
    key: u32,
    vers: Vers,
    p_limit: PLimit,
    key_info: KeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

const _: () = assert!(std::mem::size_of::<KeyData>() == 80);

pub fn fourcc(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut v = 0u32;
    for i in 0..4 {
        v = (v << 8) | (*b.get(i).unwrap_or(&b' ') as u32);
    }
    v
}

pub fn fourcc_str(v: u32) -> String {
    let bytes = v.to_be_bytes();
    bytes.iter().map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }).collect()
}

#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub size: u32,
    pub dtype: u32,
}

#[derive(Debug)]
pub enum SmcError {
    Open(i32),
    Call(i32),
    Result(u8),
    Unsupported(String),
}

impl std::fmt::Display for SmcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmcError::Open(c) => write!(f, "impossible d'ouvrir AppleSMC (kern 0x{:x})", c),
            SmcError::Call(c) => {
                if *c as u32 == 0xe00002c1 {
                    write!(f, "accès refusé par le SMC (le moteur doit tourner en root)")
                } else {
                    write!(f, "appel SMC échoué (kern 0x{:x})", *c as u32)
                }
            }
            SmcError::Result(r) => match r {
                132 => write!(f, "clé SMC introuvable"),
                _ => write!(f, "le SMC a renvoyé le code {}", r),
            },
            SmcError::Unsupported(t) => write!(f, "type SMC non supporté: {}", t),
        }
    }
}

pub struct Smc {
    conn: IoConnect,
    cache: Mutex<HashMap<u32, Info>>,
}

unsafe impl Send for Smc {}
unsafe impl Sync for Smc {}

impl Smc {
    pub fn open() -> Result<Smc, SmcError> {
        unsafe {
            let name = CString::new("AppleSMC").unwrap();
            let matching = IOServiceMatching(name.as_ptr());
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return Err(SmcError::Open(-1));
            }
            let mut conn: IoConnect = 0;
            let kr = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
            IOObjectRelease(service);
            if kr != 0 {
                return Err(SmcError::Open(kr));
            }
            Ok(Smc { conn, cache: Mutex::new(HashMap::new()) })
        }
    }

    fn call(&self, input: &KeyData) -> Result<KeyData, SmcError> {
        let mut out = KeyData::default();
        let mut out_size = std::mem::size_of::<KeyData>();
        let kr = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                input as *const _ as *const c_void,
                std::mem::size_of::<KeyData>(),
                &mut out as *mut _ as *mut c_void,
                &mut out_size,
            )
        };
        if kr != 0 {
            return Err(SmcError::Call(kr));
        }
        if out.result != 0 {
            return Err(SmcError::Result(out.result));
        }
        Ok(out)
    }

    pub fn key_info(&self, key: u32) -> Result<Info, SmcError> {
        if let Some(i) = self.cache.lock().unwrap().get(&key) {
            return Ok(*i);
        }
        let mut input = KeyData::default();
        input.key = key;
        input.data8 = CMD_READ_KEYINFO;
        let out = self.call(&input)?;
        let info = Info { size: out.key_info.data_size, dtype: out.key_info.data_type };
        self.cache.lock().unwrap().insert(key, info);
        Ok(info)
    }

    pub fn read_raw(&self, key: &str) -> Result<(Info, Vec<u8>), SmcError> {
        let k = fourcc(key);
        let info = self.key_info(k)?;
        let mut input = KeyData::default();
        input.key = k;
        input.key_info.data_size = info.size;
        input.data8 = CMD_READ_BYTES;
        let out = self.call(&input)?;
        let n = (info.size as usize).min(32);
        Ok((info, out.bytes[..n].to_vec()))
    }

    pub fn read_f64(&self, key: &str) -> Result<f64, SmcError> {
        let (info, bytes) = self.read_raw(key)?;
        decode(info.dtype, &bytes)
    }

    pub fn write_f64(&self, key: &str, value: f64) -> Result<(), SmcError> {
        let k = fourcc(key);
        let info = self.key_info(k)?;
        let bytes = encode(info.dtype, info.size, value)?;
        let mut input = KeyData::default();
        input.key = k;
        input.data8 = CMD_WRITE_BYTES;
        input.key_info.data_size = info.size;
        input.bytes[..bytes.len()].copy_from_slice(&bytes);
        self.call(&input)?;
        Ok(())
    }

    /// Écrit des octets bruts (la taille doit correspondre exactement à celle de la clé).
    pub fn write_raw(&self, key: &str, bytes: &[u8]) -> Result<(), SmcError> {
        let k = fourcc(key);
        let info = self.key_info(k)?;
        if info.size as usize != bytes.len() || bytes.len() > 32 {
            return Err(SmcError::Unsupported(format!("{key}: taille {} attendue, {} fournie", info.size, bytes.len())));
        }
        let mut input = KeyData::default();
        input.key = k;
        input.data8 = CMD_WRITE_BYTES;
        input.key_info.data_size = info.size;
        input.bytes[..bytes.len()].copy_from_slice(bytes);
        self.call(&input)?;
        Ok(())
    }

    pub fn has_key(&self, key: &str) -> bool {
        self.key_info(fourcc(key)).is_ok()
    }

    pub fn key_count(&self) -> u32 {
        self.read_f64("#KEY").map(|v| v as u32).unwrap_or(0)
    }

    pub fn key_at(&self, index: u32) -> Result<String, SmcError> {
        let mut input = KeyData::default();
        input.data8 = CMD_READ_INDEX;
        input.data32 = index;
        let out = self.call(&input)?;
        Ok(fourcc_str(out.key))
    }

    pub fn all_keys(&self) -> Vec<String> {
        (0..self.key_count()).filter_map(|i| self.key_at(i).ok()).collect()
    }

    pub fn type_of(&self, key: &str) -> Option<String> {
        self.key_info(fourcc(key)).ok().map(|i| fourcc_str(i.dtype))
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.conn);
        }
    }
}

fn decode(dtype: u32, b: &[u8]) -> Result<f64, SmcError> {
    let t = fourcc_str(dtype);
    let be16 = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]);
    let v = match t.as_str() {
        "flt " if b.len() >= 4 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        "ui8 " | "flag" if !b.is_empty() => b[0] as f64,
        "si8 " if !b.is_empty() => b[0] as i8 as f64,
        "ui16" if b.len() >= 2 => be16(b) as f64,
        "si16" if b.len() >= 2 => be16(b) as i16 as f64,
        "ui32" if b.len() >= 4 => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64,
        "fpe2" if b.len() >= 2 => be16(b) as f64 / 4.0,
        "sp78" if b.len() >= 2 => be16(b) as i16 as f64 / 256.0,
        "sp87" if b.len() >= 2 => be16(b) as i16 as f64 / 128.0,
        "sp96" if b.len() >= 2 => be16(b) as i16 as f64 / 64.0,
        "sp5a" if b.len() >= 2 => be16(b) as i16 as f64 / 1024.0,
        "ioft" if b.len() >= 8 => {
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f64 / 65536.0
        }
        _ => return Err(SmcError::Unsupported(t)),
    };
    Ok(v)
}

fn encode(dtype: u32, size: u32, v: f64) -> Result<Vec<u8>, SmcError> {
    let t = fourcc_str(dtype);
    let out = match t.as_str() {
        "flt " => (v as f32).to_le_bytes().to_vec(),
        "ui8 " | "flag" => vec![v.clamp(0.0, 255.0) as u8],
        "ui16" => (v.clamp(0.0, 65535.0) as u16).to_be_bytes().to_vec(),
        "ui32" => (v.max(0.0) as u32).to_be_bytes().to_vec(),
        "fpe2" => ((v * 4.0) as u16).to_be_bytes().to_vec(),
        _ => return Err(SmcError::Unsupported(t)),
    };
    if out.len() != size as usize {
        return Err(SmcError::Unsupported(format!("{} (taille {})", t, size)));
    }
    Ok(out)
}
