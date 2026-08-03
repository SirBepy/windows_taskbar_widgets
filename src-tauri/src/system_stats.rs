use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use sysinfo::{Disks, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize, Default)]
pub struct DiskInfo {
    pub name: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Serialize, Default)]
pub struct GpuStats {
    pub util_pct: u32,
    pub temp_c: u32,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
}

#[derive(Clone, Serialize, Default)]
pub struct SystemStats {
    pub cpu_pct: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub disks: Vec<DiskInfo>,
    pub gpu: Option<GpuStats>,
    pub cpu_temp_c: Option<f32>,
}

pub struct StatsState(pub Mutex<SystemStats>);

const DISK_REFRESH_EVERY: u32 = 15;

#[tauri::command]
pub fn get_system_stats(state: tauri::State<StatsState>) -> SystemStats {
    state.0.lock().map(|s| s.clone()).unwrap_or_default()
}

pub fn spawn_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut sys = System::new();
        let mut disks_cache: Vec<DiskInfo> = Vec::new();
        let mut cycle: u32 = 0;

        #[cfg(target_os = "windows")]
        let nvml = nvml_wrapper::Nvml::init().ok();
        #[cfg(target_os = "windows")]
        let wmi_con = wmi::COMLibrary::new()
            .ok()
            .and_then(|com| wmi::WMIConnection::with_namespace_path("root\\WMI", com).ok());

        loop {
            sys.refresh_cpu_usage();
            sys.refresh_memory();

            if cycle % DISK_REFRESH_EVERY == 0 {
                let disks = Disks::new_with_refreshed_list();
                disks_cache = disks
                    .list()
                    .iter()
                    .map(|d| DiskInfo {
                        name: d.mount_point().to_string_lossy().to_string(),
                        free_bytes: d.available_space(),
                        total_bytes: d.total_space(),
                    })
                    .collect();
            }
            cycle = cycle.wrapping_add(1);

            #[cfg(target_os = "windows")]
            let gpu = nvml.as_ref().and_then(read_gpu);
            #[cfg(not(target_os = "windows"))]
            let gpu: Option<GpuStats> = None;

            #[cfg(target_os = "windows")]
            let cpu_temp_c = wmi_con.as_ref().and_then(read_cpu_temp);
            #[cfg(not(target_os = "windows"))]
            let cpu_temp_c: Option<f32> = None;

            let stats = SystemStats {
                cpu_pct: sys.global_cpu_usage(),
                mem_used_bytes: sys.used_memory(),
                mem_total_bytes: sys.total_memory(),
                disks: disks_cache.clone(),
                gpu,
                cpu_temp_c,
            };

            if let Some(state) = app.try_state::<StatsState>() {
                if let Ok(mut latest) = state.0.lock() {
                    *latest = stats.clone();
                }
            }
            let _ = app.emit("system-stats", &stats);

            let poll_s = app
                .try_state::<crate::settings::SettingsState>()
                .and_then(|s| s.0.lock().ok().map(|s| s.stats_poll_seconds))
                .unwrap_or(2)
                .max(1);
            std::thread::sleep(std::time::Duration::from_secs(poll_s as u64));
        }
    });
}

#[derive(Clone, Serialize)]
pub struct ProcRow {
    pub name: String,
    pub pct_or_bytes: f64,
    pub count: u32,
}

// Dedicated instance kept warm across calls: consecutive refreshes need to see the
// same process set to compute a meaningful cpu% delta. Only touched on hover, never
// from the 2s poller (a full process refresh is comparatively expensive).
static PROC_SYS: OnceLock<Mutex<System>> = OnceLock::new();

#[tauri::command]
pub fn get_top_processes(metric: String) -> Vec<ProcRow> {
    let sys_mutex = PROC_SYS.get_or_init(|| Mutex::new(System::new()));
    let Ok(mut sys) = sys_mutex.lock() else { return Vec::new() };

    sys.refresh_processes(ProcessesToUpdate::All, true);
    if metric == "cpu" {
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_processes(ProcessesToUpdate::All, true);
    }

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

#[cfg(target_os = "windows")]
fn read_gpu(nvml: &nvml_wrapper::Nvml) -> Option<GpuStats> {
    let dev = nvml.device_by_index(0).ok()?;
    let util = dev.utilization_rates().ok()?;
    let mem = dev.memory_info().ok()?;
    let temp = dev
        .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
        .unwrap_or(0);
    Some(GpuStats {
        util_pct: util.gpu,
        temp_c: temp,
        vram_used_bytes: mem.used,
        vram_total_bytes: mem.total,
    })
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

// MSAcpi thermal zones report tenths of Kelvin; many consumer boards don't expose them
// at all (query errors or returns nothing), so this silently degrades to None.
#[cfg(target_os = "windows")]
fn read_cpu_temp(con: &wmi::WMIConnection) -> Option<f32> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(rename = "MSAcpi_ThermalZoneTemperature", rename_all = "PascalCase")]
    struct ThermalZone {
        current_temperature: u32,
    }
    let zones: Vec<ThermalZone> = con.query().ok()?;
    zones
        .iter()
        .map(|z| z.current_temperature as f32 / 10.0 - 273.15)
        .filter(|c| *c > 0.0 && *c < 120.0)
        .fold(None, |acc: Option<f32>, c| Some(acc.map_or(c, |a| a.max(c))))
}
