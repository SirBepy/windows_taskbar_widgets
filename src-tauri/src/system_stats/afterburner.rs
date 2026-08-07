// MSI Afterburner's MAHM shared-memory layout: header (dwHeaderSize bytes) then
// dwNumEntries fixed-size entries, each five 260-byte NUL-terminated ASCII strings
// (szSrcName first) followed by f32 data/minLimit/maxLimit and three trailing u32s.
const SIGNATURE: u32 = 0x4D41484D;
const NAME_FIELD_LEN: usize = 260;
const DATA_OFFSET_IN_ENTRY: usize = NAME_FIELD_LEN * 5;
const TARGET_NAME: &[u8] = b"CPU temperature";

fn read_u32(buf: &[u8], offset: usize) -> Option<u32> {
    buf.get(offset..offset + 4)?.try_into().ok().map(u32::from_le_bytes)
}

/// Finds the "CPU temperature" entry in a raw MAHM snapshot and returns its value.
/// Pure and Windows-API-free so it's testable against a synthetic buffer.
fn parse_cpu_temp(buf: &[u8]) -> Option<f32> {
    if read_u32(buf, 0)? != SIGNATURE {
        return None; // covers 0xDEAD (shutting down) and any garbage/absent mapping
    }
    let header_size = read_u32(buf, 8)? as usize;
    let num_entries = read_u32(buf, 12)? as usize;
    let entry_size = read_u32(buf, 16)? as usize;
    if entry_size < DATA_OFFSET_IN_ENTRY + 4 {
        return None;
    }
    let total = num_entries.checked_mul(entry_size)?.checked_add(header_size)?;
    if buf.len() < total {
        return None;
    }

    for i in 0..num_entries {
        let entry = header_size + i * entry_size;
        let name = &buf[entry..entry + NAME_FIELD_LEN];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
        if name != TARGET_NAME {
            continue;
        }
        let data_at = entry + DATA_OFFSET_IN_ENTRY;
        let value = f32::from_le_bytes(buf[data_at..data_at + 4].try_into().ok()?);
        return (0.0..=150.0).contains(&value).then_some(value);
    }
    None
}

/// Reads the live CPU temperature out of Afterburner's `MAHMSharedMemory` mapping.
/// Returns `None` quietly whenever Afterburner isn't running; that's expected, not an error.
#[cfg(target_os = "windows")]
pub(crate) fn read_cpu_temp() -> Option<f32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{
        MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
        MEMORY_BASIC_INFORMATION,
    };

    let name: Vec<u16> = "MAHMSharedMemory\0".encode_utf16().collect();
    // SAFETY: name is NUL-terminated; handle is null-checked before any other use.
    let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
    if handle.is_null() {
        return None;
    }

    // SAFETY: handle was just validated non-null; view.Value is checked before use.
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0) };
    let result = (!view.Value.is_null()).then(|| {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: view.Value is a valid mapped address; mbi is sized for the call.
        let queried = unsafe {
            VirtualQuery(view.Value, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>())
        };
        (queried != 0 && mbi.RegionSize > 0).then(|| {
            // SAFETY: RegionSize is the actually-committed span VirtualQuery reports
            // for this base address, so the slice never reads past the real mapping.
            let slice = unsafe { std::slice::from_raw_parts(view.Value as *const u8, mbi.RegionSize) };
            parse_cpu_temp(slice)
        })
    });

    unsafe {
        if !view.Value.is_null() {
            UnmapViewOfFile(view);
        }
        CloseHandle(handle);
    }
    result.flatten().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_SIZE: u32 = 32;
    const ENTRY_SIZE: u32 = 1324;

    fn build(entries: &[(&str, f32)]) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE as usize];
        buf[0..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        buf[8..12].copy_from_slice(&HEADER_SIZE.to_le_bytes());
        buf[12..16].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        buf[16..20].copy_from_slice(&ENTRY_SIZE.to_le_bytes());

        for (name, value) in entries {
            let mut entry = vec![0u8; ENTRY_SIZE as usize];
            entry[..name.len()].copy_from_slice(name.as_bytes());
            entry[DATA_OFFSET_IN_ENTRY..DATA_OFFSET_IN_ENTRY + 4]
                .copy_from_slice(&value.to_le_bytes());
            buf.extend_from_slice(&entry);
        }
        buf
    }

    #[test]
    fn finds_cpu_temperature_by_name() {
        let buf = build(&[("GPU temperature", 55.0), ("CPU temperature", 68.0)]);
        assert_eq!(parse_cpu_temp(&buf), Some(68.0));
    }

    #[test]
    fn shutdown_signature_is_rejected() {
        let mut buf = build(&[("CPU temperature", 68.0)]);
        buf[0..4].copy_from_slice(&0xDEADu32.to_le_bytes());
        assert_eq!(parse_cpu_temp(&buf), None);
    }

    #[test]
    fn missing_name_returns_none() {
        let buf = build(&[("GPU temperature", 55.0), ("Board power", 120.0)]);
        assert_eq!(parse_cpu_temp(&buf), None);
    }

    #[test]
    fn out_of_range_value_is_rejected() {
        let buf = build(&[("CPU temperature", 400.0)]);
        assert_eq!(parse_cpu_temp(&buf), None);
    }

    #[test]
    fn truncated_buffer_returns_none() {
        let buf = build(&[("CPU temperature", 68.0)]);
        assert_eq!(parse_cpu_temp(&buf[..buf.len() - 10]), None);
    }

    // Reads the live MAHM mapping; needs Afterburner actually running, so it's excluded
    // from the normal `cargo test` run.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires MSI Afterburner running locally"]
    fn live_read_returns_a_plausible_temperature() {
        let value = read_cpu_temp().expect("Afterburner must be running for this test");
        println!("live CPU temperature: {value}");
        assert!((0.0..=150.0).contains(&value));
    }
}
