use serde::Serialize;
use std::sync::Mutex;
use sysinfo::{Disks, System};
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
