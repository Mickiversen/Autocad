//! DWG support through a DXF bridge.
//!
//! DWG files are read with the `acadrust` crate (MPL-2.0, used unmodified as a dependency) and
//! converted to DXF bytes, which CADCraft's own DXF reader maps to its document model; saving
//! goes the other way. Keeping the dependency behind this byte-level API isolates it: nothing
//! else in CADCraft depends on its types. Also builds for wasm32 (it reads from in-memory buffers).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

/// DWG version codes we can write.
pub const VERSIONS: &[(&str, &str)] =
    &[("AC1015", "2000"), ("AC1018", "2004"), ("AC1021", "2007"), ("AC1024", "2010"), ("AC1027", "2013"), ("AC1032", "2018")];

/// True when the bytes look like a DWG file (`AC10xx` magic).
pub fn is_dwg(bytes: &[u8]) -> bool {
    bytes.len() > 6 && bytes.starts_with(b"AC10") && bytes.get(4..6).is_some_and(|v| v.iter().all(u8::is_ascii_digit))
}

/// The DWG version code of a file (e.g. "AC1032").
pub fn version(bytes: &[u8]) -> Option<String> {
    is_dwg(bytes).then(|| String::from_utf8_lossy(bytes.get(0..6).unwrap_or_default()).to_string())
}

mod native {
    use std::io::Cursor;

    /// Read DWG bytes and return an ASCII DXF rendition.
    pub fn dwg_to_dxf(bytes: &[u8]) -> Result<Vec<u8>, String> {
        let data = bytes.to_vec();
        let doc = std::panic::catch_unwind(move || acadrust::DwgReader::from_stream(Cursor::new(data)).read())
            .map_err(|_| "the DWG reader failed on this file".to_string())?
            .map_err(|e| format!("DWG: {e}"))?;
        acadrust::DxfWriter::new(&doc).write_to_vec().map_err(|e| format!("DWG→DXF: {e}"))
    }

    /// Convert DXF bytes into a DWG file.
    pub fn dxf_to_dwg(dxf: &[u8]) -> Result<Vec<u8>, String> {
        let data = dxf.to_vec();
        let doc = std::panic::catch_unwind(move || acadrust::DxfReader::from_reader(Cursor::new(data)).and_then(|r| r.read()))
            .map_err(|_| "the DXF→DWG conversion failed".to_string())?
            .map_err(|e| format!("DXF: {e}"))?;
        acadrust::DwgWriter::write_to_vec(&doc).map_err(|e| format!("DWG write: {e}"))
    }
}

pub use native::{dwg_to_dxf, dxf_to_dwg};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic() {
        assert!(is_dwg(b"AC1032\0\0\0\0"));
        assert!(!is_dwg(b"  0\r\nSECTION"));
        assert_eq!(version(b"AC1018xxxx").as_deref(), Some("AC1018"));
    }

    #[test]
    fn garbage_is_an_error_not_a_crash() {
        assert!(dwg_to_dxf(b"AC1032 this is not a dwg file at all").is_err());
        assert!(dwg_to_dxf(b"").is_err());
    }
}
