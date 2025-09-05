use std::{
    fs::{self, File},
    io::Write,
    os::unix::io::AsRawFd,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{anyhow, Result};
use nix::errno::Errno;

const TDX_DEVICE_PATHS: &[&str] = &["/dev/tdx_guest", "/dev/tdx-guest"];

// Sizes from TDX UAPI
const TDREPORT_SIZE: usize = 1024;
const TDX_REPORT_DATA_SIZE: usize = 64;

// Thread-safe counter for unique TSM directory names
static TSM_COUNTER: AtomicU64 = AtomicU64::new(0);

// ===== Kernel UAPI structs / ioctls =====

#[repr(C)]
struct TdxReportReq {
    reportdata: [u8; TDX_REPORT_DATA_SIZE],
    tdreport: [u8; TDREPORT_SIZE],
}

#[repr(C)]
struct TdxQuoteHdr {
    version: u64, // set to 1
    status: u64,  // kernel/VMM fills
    in_len: u32,  // TDREPORT_SIZE (1024)
    out_len: u32, // kernel/VMM fills
}

#[repr(C)]
struct TdxQuoteReq {
    buf: u64, // userspace VA of (TdxQuoteHdr + data[])
    len: u64, // total buffer length
}

nix::ioctl_readwrite!(tdx_get_report0, b'T', 1, TdxReportReq);
nix::ioctl_readwrite!(tdx_get_quote, b'T', 2, TdxQuoteReq);

// ===== Public interface =====

pub trait TdxInterface: Send + Sync {
    fn is_hardware_available(&self) -> bool;
    fn generate_quote(&self, report_data: &[u8]) -> Result<Vec<u8>>;
}

// ===== Real hardware implementation =====

pub struct TdxHardware {
    dev_file: Option<File>,
}

impl Default for TdxHardware {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxHardware {
    pub fn new() -> Self {
        Self { dev_file: None }
    }

    fn in_tdx_guest() -> bool {
        // Check multiple possible indicators of TDX guest environment
        Path::new("/sys/firmware/tdx_guest").exists()
            || Path::new("/sys/kernel/config/tsm/report").exists()
            || TDX_DEVICE_PATHS.iter().any(|p| Path::new(p).exists())
    }

    fn open_device(&mut self) -> Result<i32> {
        if let Some(f) = &self.dev_file {
            return Ok(f.as_raw_fd());
        }
        for p in TDX_DEVICE_PATHS {
            if let Ok(f) = File::options().read(true).write(true).open(p) {
                // ensure it is a char device
                if f.metadata().ok().is_some_and(|m| {
                    use std::os::unix::fs::FileTypeExt;
                    m.file_type().is_char_device()
                }) {
                    let fd = f.as_raw_fd();
                    self.dev_file = Some(f);
                    return Ok(fd);
                }
            }
        }
        Err(anyhow!(
            "TDX device not found. Tried: {}",
            TDX_DEVICE_PATHS.join(", ")
        ))
    }

    // Preferred path: configfs TSM
    fn quote_via_tsm(&self, report_data: [u8; 64]) -> Result<Vec<u8>> {
        let cfg = Path::new("/sys/kernel/config");
        if !cfg.exists() {
            return Err(anyhow!(
                "configfs not mounted: mount -t configfs none /sys/kernel/config"
            ));
        }
        let base = Path::new("/sys/kernel/config/tsm/report");
        if !base.exists() {
            // Try to create the subtree; requires CAP_SYS_ADMIN
            fs::create_dir_all(base)
                .map_err(|e| anyhow!("create {} failed: {e}", base.display()))?;
        }

        let unique_id = TSM_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = base.join(format!("report{}_{}", std::process::id(), unique_id));
        // Best-effort cleanup guard
        let cleanup = |d: &std::path::Path| {
            let _ = fs::remove_file(d.join("inblob"));
            let _ = fs::remove_file(d.join("outblob"));
            let _ = fs::remove_dir(d);
        };

        fs::create_dir(&dir).map_err(|e| anyhow!("mkdir {}: {e}", dir.display()))?;

        let res = (|| -> Result<Vec<u8>> {
            // write 64B REPORTDATA
            let mut inblob =
                File::create(dir.join("inblob")).map_err(|e| anyhow!("open inblob: {e}"))?;
            inblob
                .write_all(&report_data)
                .map_err(|e| anyhow!("write inblob: {e}"))?;
            drop(inblob);

            // read quote
            let quote = fs::read(dir.join("outblob")).map_err(|e| anyhow!("read outblob: {e}"))?;
            if quote.is_empty() {
                return Err(anyhow!("TSM outblob empty"));
            }
            Ok(quote)
        })();

        cleanup(&dir);
        res
    }

    // Legacy path: GET_REPORT0 ioctl + GET_QUOTE ioctl (if supported)
    fn quote_via_legacy(&mut self, report_data: [u8; 64]) -> Result<Vec<u8>> {
        let fd = self.open_device()?;

        // 1) TDREPORT
        let mut req = TdxReportReq {
            reportdata: report_data,
            tdreport: [0u8; TDREPORT_SIZE],
        };
        unsafe {
            tdx_get_report0(fd, &mut req).map_err(|e| match e {
                Errno::ENOTTY => anyhow!("GET_REPORT0 not supported by this kernel/device"),
                Errno::EOPNOTSUPP => anyhow!("GET_REPORT0 not supported by VMM/hypervisor"),
                Errno::ENOSYS => anyhow!("GET_REPORT0 not implemented"),
                _ => anyhow!("GET_REPORT0 failed: {e}"),
            })?;
        }

        // 2) GET_QUOTE (may not exist)
        let hdr_len = std::mem::size_of::<TdxQuoteHdr>();
        let data_off = (hdr_len + 7) & !7; // 8B align
        let mut buf = vec![0u8; 16 * 1024];

        {
            let hdr = unsafe { &mut *(buf.as_mut_ptr() as *mut TdxQuoteHdr) };
            hdr.version = 1;
            hdr.status = 0;
            hdr.in_len = TDREPORT_SIZE as u32;
            hdr.out_len = 0;
        }
        buf[data_off..data_off + TDREPORT_SIZE].copy_from_slice(&req.tdreport);

        let mut qreq = TdxQuoteReq {
            buf: buf.as_mut_ptr() as u64,
            len: buf.len() as u64,
        };
        unsafe {
            tdx_get_quote(fd, &mut qreq).map_err(|e| match e {
                Errno::ENOTTY => anyhow!("GET_QUOTE not supported by this kernel/device"),
                Errno::EOPNOTSUPP => anyhow!("GET_QUOTE not supported by VMM/QGS"),
                Errno::ENOSYS => anyhow!("GET_QUOTE not implemented"),
                _ => anyhow!("GET_QUOTE failed: {e}"),
            })?;
        }

        let hdr = unsafe { &*(buf.as_ptr() as *const TdxQuoteHdr) };
        let qlen = hdr.out_len as usize;
        if qlen == 0 || qlen > buf.len() - data_off {
            return Err(anyhow!("invalid quote length {qlen}"));
        }
        Ok(buf[data_off..data_off + qlen].to_vec())
    }

    fn simple_quote_sanity(quote: &[u8]) -> Result<()> {
        // Minimal structural checks: version + tee_type at fixed offsets
        if quote.len() < 48 {
            return Err(anyhow!("quote too small: {}", quote.len()));
        }
        let version = u16::from_le_bytes([quote[0], quote[1]]);
        let tee_type = u32::from_le_bytes([quote[4], quote[5], quote[6], quote[7]]);
        if version != 4 && version != 5 {
            return Err(anyhow!("unexpected quote version: {}", version));
        }
        if tee_type != 0x0000_0081 {
            return Err(anyhow!(
                "unexpected TEE type 0x{:08x} (expect 0x00000081 for TDX)",
                tee_type
            ));
        }
        Ok(())
    }
}

impl TdxInterface for TdxHardware {
    fn is_hardware_available(&self) -> bool {
        let in_tdx = Self::in_tdx_guest();
        let device_available = TDX_DEVICE_PATHS.iter().any(|p| Path::new(p).exists());
        tracing::info!(
            "TDX hardware detection: in_tdx_guest={}, device_available={}",
            in_tdx,
            device_available
        );

        if !in_tdx {
            tracing::warn!("Not in TDX guest environment");
            return false;
        }
        device_available
    }

    fn generate_quote(&self, report_data: &[u8]) -> Result<Vec<u8>> {
        tracing::info!(
            "Generating TDX quote with {} bytes of report data",
            report_data.len()
        );

        if report_data.len() > TDX_REPORT_DATA_SIZE {
            return Err(anyhow!(
                "report_data too large: {} > {}",
                report_data.len(),
                TDX_REPORT_DATA_SIZE
            ));
        }
        let mut rd = [0u8; 64];
        rd[..report_data.len()].copy_from_slice(report_data);

        // Need &mut self for legacy path; clone a fresh handle holder.
        let mut hw = TdxHardware::new();

        // Try TSM first
        tracing::info!("Attempting quote generation via TSM configfs");
        match hw.quote_via_tsm(rd) {
            Ok(q) => {
                tracing::info!(
                    "Successfully generated quote via TSM, length: {} bytes",
                    q.len()
                );
                Self::simple_quote_sanity(&q)?;
                Ok(q)
            }
            Err(e) => {
                tracing::warn!("TSM quote generation failed: {}", e);
                // Fall back to legacy only if device exists
                if !TDX_DEVICE_PATHS.iter().any(|p| Path::new(p).exists()) {
                    return Err(anyhow!("TSM failed: {e}"));
                }
                // Legacy
                tracing::info!("Attempting quote generation via legacy ioctl");
                let q = hw.quote_via_legacy(rd)?;
                tracing::info!(
                    "Successfully generated quote via legacy ioctl, length: {} bytes",
                    q.len()
                );
                Self::simple_quote_sanity(&q)?;
                Ok(q)
            }
        }
    }
}

// ===== Simple simulator (no SGX dependencies) =====

pub struct TdxSimulator {
    cfg: TdxSimulatorConfig,
}

#[derive(Clone, Debug)]
pub struct TdxSimulatorConfig {
    pub mrtd: [u8; 48],
    pub td_attributes: [u8; 8],
    pub debug_enabled: bool,
    pub security_version: u32,
    pub quote_version: u16, // 4 or 5
}

impl Default for TdxSimulatorConfig {
    fn default() -> Self {
        Self {
            mrtd: [0x42; 48],
            td_attributes: [0x00; 8],
            debug_enabled: false,
            security_version: 0,
            quote_version: 4,
        }
    }
}

impl Default for TdxSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxSimulator {
    pub fn new() -> Self {
        Self {
            cfg: TdxSimulatorConfig::default(),
        }
    }
    pub fn new_with_config(cfg: TdxSimulatorConfig) -> Self {
        Self { cfg }
    }

    fn make_mock_quote(&self, report_data: &[u8]) -> Vec<u8> {
        use rand::Rng;
        
        let mut q = Vec::new();
        let mut rng = rand::thread_rng();

        // Quote header (48 bytes)
        q.extend_from_slice(&self.cfg.quote_version.to_le_bytes()); // version (2 bytes)
        q.extend_from_slice(&[0x02, 0x00]); // att_key_type = ECDSA (2 bytes)
        q.extend_from_slice(&0x00000081u32.to_le_bytes()); // tee_type TDX (4 bytes)
        
        // Randomize reserved (4 bytes)
        let mut reserved = [0u8; 4];
        rng.fill(&mut reserved[..]);
        q.extend_from_slice(&reserved);
        
        // Randomize qe_vendor_id (16 bytes)
        let mut qe_vendor_id = [0u8; 16];
        rng.fill(&mut qe_vendor_id[..]);
        q.extend_from_slice(&qe_vendor_id);

        // first 20 bytes of report_data (20 bytes) - randomize unused portion
        let mut user20 = [0u8; 20];
        let n = user20.len().min(report_data.len());
        user20[..n].copy_from_slice(&report_data[..n]);
        if n < 20 {
            rng.fill(&mut user20[n..]);
        }
        q.extend_from_slice(&user20);

        // Fake TD report body (584 bytes) - proper TDX structure
        let mut body = vec![0u8; 584];

        // Randomize TCB SVN (16 bytes) at offset 0
        let mut tcb_svn = [0u8; 16];
        rng.fill(&mut tcb_svn[..]);
        body[0..16].copy_from_slice(&tcb_svn);

        // Randomize MR_SEAM (48 bytes) at offset 16
        let mut mr_seam = [0u8; 48];
        rng.fill(&mut mr_seam[..]);
        body[16..64].copy_from_slice(&mr_seam);

        // Randomize MR_SIGNER_SEAM (48 bytes) at offset 64
        let mut mr_signer_seam = [0u8; 48];
        rng.fill(&mut mr_signer_seam[..]);
        body[64..112].copy_from_slice(&mr_signer_seam);

        // Randomize SEAM attributes (8 bytes) at offset 112
        let mut seam_attrs = [0u8; 8];
        rng.fill(&mut seam_attrs[..]);
        body[112..120].copy_from_slice(&seam_attrs);

        // TD attributes (8 bytes) at offset 120 - randomize but preserve debug bit
        let mut td_attrs = [0u8; 8];
        rng.fill(&mut td_attrs[..]);
        if self.cfg.debug_enabled {
            td_attrs[0] |= 0x01; // Set debug bit
        } else {
            td_attrs[0] &= !0x01; // Clear debug bit
        }
        body[120..128].copy_from_slice(&td_attrs);

        // Randomize XFAM (8 bytes) at offset 128
        let mut xfam = [0u8; 8];
        rng.fill(&mut xfam[..]);
        body[128..136].copy_from_slice(&xfam);

        // MRTD (48 bytes) at offset 136 - randomize configured value or use random
        let mut mrtd = [0u8; 48];
        if self.cfg.mrtd == [0x42; 48] {
            // If using default, randomize it
            rng.fill(&mut mrtd[..]);
        } else {
            // Use configured value but add some randomness to non-critical bytes
            mrtd.copy_from_slice(&self.cfg.mrtd);
            // Randomize last few bytes to simulate measurement variations
            rng.fill(&mut mrtd[44..]);
        }
        body[136..184].copy_from_slice(&mrtd);

        // Randomize MR_CONFIG_ID (48 bytes) at offset 184
        let mut mr_config_id = [0u8; 48];
        rng.fill(&mut mr_config_id[..]);
        body[184..232].copy_from_slice(&mr_config_id);

        // Randomize MR_OWNER (48 bytes) at offset 232
        let mut mr_owner = [0u8; 48];
        rng.fill(&mut mr_owner[..]);
        body[232..280].copy_from_slice(&mr_owner);

        // Randomize MR_OWNER_CONFIG (48 bytes) at offset 280
        let mut mr_owner_config = [0u8; 48];
        rng.fill(&mut mr_owner_config[..]);
        body[280..328].copy_from_slice(&mr_owner_config);

        // Randomize RTMR 0-3 (4 * 48 bytes) at offset 328
        for i in 0..4 {
            let mut rtmr = [0u8; 48];
            rng.fill(&mut rtmr[..]);
            let rtmr_offset = 328 + (i * 48);
            body[rtmr_offset..rtmr_offset + 48].copy_from_slice(&rtmr);
        }

        // Report data (64 bytes) at offset 520
        let mut rd64 = [0u8; 64];
        let m = rd64.len().min(report_data.len());
        rd64[..m].copy_from_slice(&report_data[..m]);
        // Randomize unused portion of report data
        if m < 64 {
            rng.fill(&mut rd64[m..]);
        }
        body[520..584].copy_from_slice(&rd64);

        q.extend_from_slice(&body);

        // Randomize signature data length + signature bytes
        let mut sig_data = vec![0u8; 64 + rng.gen_range(0..128)]; // Random signature size
        rng.fill(&mut sig_data[..]);
        q.extend_from_slice(&(sig_data.len() as u32).to_le_bytes());
        q.extend_from_slice(&sig_data);

        q
    }
}

impl TdxInterface for TdxSimulator {
    fn is_hardware_available(&self) -> bool {
        tracing::info!("TDX simulator always reports hardware available");
        true
    }

    fn generate_quote(&self, report_data: &[u8]) -> Result<Vec<u8>> {
        tracing::info!(
            "Generating SIMULATED TDX quote with {} bytes of report data",
            report_data.len()
        );
        if report_data.len() > TDX_REPORT_DATA_SIZE {
            return Err(anyhow!(
                "report_data too large: {} > {}",
                report_data.len(),
                TDX_REPORT_DATA_SIZE
            ));
        }
        Ok(self.make_mock_quote(report_data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity_simulator_quote() {
        let sim = TdxSimulator::new();
        let q = sim.generate_quote(b"hello tdx").unwrap();
        assert!(q.len() > 600);
        assert_eq!(&q[0..2], &4u16.to_le_bytes()); // version 4 default
        assert_eq!(&q[2..4], &[0x02, 0x00]); // ECDSA
        assert_eq!(&q[4..8], &0x00000081u32.to_le_bytes()); // TDX tee_type
    }

    #[test]
    fn hardware_detection_is_boolean() {
        let hw = TdxHardware::new();
        let _ = hw.is_hardware_available(); // should not panic
                                            // Test that TdxSimulator can be created
    }
}
