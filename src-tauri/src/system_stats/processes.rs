use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Serialize)]
pub struct ProcRow {
    pub name: String,
    pub pct_or_bytes: f64,
    pub count: u32,
}

// Dedicated instance kept warm across calls: consecutive refreshes need to see the
// same process set to compute a meaningful cpu% delta. Only touched on hover, never
// from the 2s poller (a full process refresh is comparatively expensive).
static PROC_SYS: OnceLock<Mutex<(System, Option<Instant>)>> = OnceLock::new();

// Past this, the warm instance's previous refresh is too old to be a cpu% baseline
// (it would report the average since then), so a fresh pair has to be taken.
const PROC_BASELINE_MAX_AGE: Duration = Duration::from_secs(10);

#[tauri::command]
pub fn get_top_processes(metric: String) -> Vec<ProcRow> {
    let sys_mutex = PROC_SYS.get_or_init(|| Mutex::new((System::new(), None)));
    let Ok(mut guard) = sys_mutex.lock() else { return Vec::new() };
    let (sys, last_refresh) = &mut *guard;

    // Names and the chosen metric only; the default refreshes exe/cmd/cwd/environ/
    // user for every process on the box, none of which is read below.
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
    // A flyout polls us every 2s, so the throwaway refresh + 200ms block is paid
    // only on a cold first hover rather than on every call.
    let stale = last_refresh.is_none_or(|t| t.elapsed() > PROC_BASELINE_MAX_AGE);
    if metric == "cpu" && stale {
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    }
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    *last_refresh = Some(Instant::now());

    if metric == "gpu" {
        #[cfg(target_os = "windows")]
        let by_pid = read_gpu_engine_by_pid().unwrap_or_default();
        #[cfg(not(target_os = "windows"))]
        let by_pid: HashMap<u32, f64> = HashMap::new();

        let mut grouped: HashMap<String, (f64, u32)> = HashMap::new();
        for (pid, pct) in by_pid {
            let Some(proc) = sys.process(sysinfo::Pid::from_u32(pid)) else { continue };
            let entry = grouped.entry(proc.name().to_string_lossy().to_string()).or_insert((0.0, 0));
            entry.0 += pct;
            entry.1 += 1;
        }
        return top5(grouped, |sum| sum);
    }

    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;

    let mut grouped: HashMap<String, (f64, u32)> = HashMap::new();
    for proc in sys.processes().values() {
        let name = proc.name().to_string_lossy().to_string();
        let value = if metric == "ram" {
            proc.memory() as f64
        } else {
            proc.cpu_usage() as f64
        };
        let entry = grouped.entry(name).or_insert((0.0, 0));
        entry.0 += value;
        entry.1 += 1;
    }

    top5(grouped, |sum| if metric == "cpu" { sum / cores } else { sum })
}

/// Shared tail of every metric: scale the summed value, rank desc, keep the top 5.
fn top5(grouped: HashMap<String, (f64, u32)>, scale: impl Fn(f64) -> f64) -> Vec<ProcRow> {
    let mut rows: Vec<ProcRow> = grouped
        .into_iter()
        .map(|(name, (sum, count))| ProcRow { name, pct_or_bytes: scale(sum), count })
        .collect();
    rows.sort_by(|a, b| b.pct_or_bytes.total_cmp(&a.pct_or_bytes));
    rows.truncate(5);
    rows
}

// Instance names look like pid_1234_luid_..._eng_0_engtype_3D; one row per engine
// per process, so the pid is everything between "pid_" and the next underscore.
#[cfg(target_os = "windows")]
fn parse_gpu_engine_pid(instance: &str) -> Option<u32> {
    instance.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

// Task Manager's own source for per-process GPU%: the "GPU Engine" PDH object, one
// instance per (pid, engine). Driver/build dependent; missing counters degrade to None.
#[cfg(target_os = "windows")]
fn read_gpu_engine_by_pid() -> Option<HashMap<u32, f64>> {
    use windows_sys::Win32::System::Performance::{
        PdhAddCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    };

    unsafe {
        let mut query: PDH_HQUERY = std::ptr::null_mut();
        if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
            return None;
        }
        let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0".encode_utf16().collect();
        let mut counter: PDH_HCOUNTER = std::ptr::null_mut();
        if PdhAddCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
            PdhCloseQuery(query);
            return None;
        }

        // Rate counter: first sample is a throwaway baseline, second (after a wait) is real.
        PdhCollectQueryData(query);
        std::thread::sleep(std::time::Duration::from_millis(200));
        if PdhCollectQueryData(query) != 0 {
            PdhCloseQuery(query);
            return None;
        }

        let mut buf_size: u32 = 0;
        let mut item_count: u32 = 0;
        let sizing = PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            std::ptr::null_mut(),
        );
        if sizing != PDH_MORE_DATA || buf_size == 0 {
            PdhCloseQuery(query);
            return Some(HashMap::new());
        }

        // Vec<u64> (not u8) so the buffer is 8-byte aligned for the item struct's f64/pointer fields.
        let mut buf: Vec<u64> = vec![0u64; buf_size.div_ceil(8) as usize];
        let item_ptr = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let r = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut buf_size, &mut item_count, item_ptr);
        PdhCloseQuery(query);
        if r != 0 {
            return None;
        }

        let items = std::slice::from_raw_parts(item_ptr as *const PDH_FMT_COUNTERVALUE_ITEM_W, item_count as usize);
        let mut by_pid: HashMap<u32, f64> = HashMap::new();
        for item in items {
            if item.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA || item.szName.is_null() {
                continue;
            }
            let len = (0..).take_while(|&i| *item.szName.add(i) != 0).count();
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len));
            let Some(pid) = parse_gpu_engine_pid(&name) else { continue };
            *by_pid.entry(pid).or_insert(0.0) += item.FmtValue.Anonymous.doubleValue;
        }
        Some(by_pid)
    }
}
