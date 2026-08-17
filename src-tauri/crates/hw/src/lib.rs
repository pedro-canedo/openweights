//! Detecção de hardware (na inicialização) e telemetria em tempo real (1–2 Hz).
//!
//! Fontes por plataforma:
//! - RAM/CPU: `sysinfo` (instância persistente; CPU% exige duas leituras
//!   separadas por >= `MINIMUM_CPU_UPDATE_INTERVAL`).
//! - AVX2/AVX-512: `is_x86_feature_detected!`.
//! - NVIDIA: NVML via `nvml-wrapper` (carregado dinamicamente — falha graciosa
//!   sem driver). VRAM/nome/driver/compute capability + utilização.
//! - Windows (qualquer vendor): DXGI `EnumAdapters1` para enumeração e VRAM
//!   total; classificação iGPU/dGPU via `D3D12_FEATURE_ARCHITECTURE1` (UMA) —
//!   nunca confiar apenas em `DedicatedVideoMemory > 0`. Telemetria: VRAM
//!   usada via `QueryVideoMemoryInfo` e utilização AMD/Intel via contadores
//!   PDH (`\GPU Engine(*)\Utilization Percentage`) — ver `windows_gpu`.
//! - macOS: memória unificada (`recommendedMaxWorkingSetSize` ≈ 75% da RAM).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lr_types::{GpuInfo, GpuTelemetry, HardwareProfile, Telemetry};
use sysinfo::{Components, DiskRefreshKind, Disks, Networks, System};

#[cfg(windows)]
mod windows_gpu;

/// Detecta o perfil completo da máquina. Chamado uma vez na inicialização
/// (e sob demanda pelo botão "redetectar" nas configurações).
pub fn detect() -> HardwareProfile {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_else(|| "CPU desconhecida".to_string());

    HardwareProfile {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_name,
        cpu_cores: sys.cpus().len(),
        avx2: detect_avx2(),
        avx512: detect_avx512(),
        ram_total_bytes: sys.total_memory(),
        gpus: detect_gpus(),
    }
}

fn detect_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn detect_avx512() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn detect_gpus() -> Vec<GpuInfo> {
    #[cfg(windows)]
    {
        windows_gpu::detect()
    }
    #[cfg(target_os = "macos")]
    {
        macos_gpu_stub()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn macos_gpu_stub() -> Vec<GpuInfo> {
    // Apple Silicon: memória unificada. O teto prático para Metal é
    // ~75% da RAM (recommendedMaxWorkingSetSize); usamos essa fração até a
    // integração com Metal ser implementada (M1).
    let mut sys = System::new();
    sys.refresh_memory();
    vec![GpuInfo {
        name: "Apple GPU (memória unificada)".to_string(),
        vendor: lr_types::GpuVendor::Apple,
        vram_total_bytes: sys.total_memory() * 3 / 4,
        is_integrated: true,
        driver_version: None,
        cuda_compute: None,
    }]
}

/// Só espaço total/livre — sem tipo de disco nem contadores de I/O, que
/// custam syscalls extras e não aparecem no payload.
fn disk_refresh_kind() -> DiskRefreshKind {
    DiskRefreshKind::nothing().with_storage()
}

/// Amostrador de telemetria. Mantenha UMA instância viva e chame
/// [`Monitor::sample`] no máximo a cada ~500 ms.
pub struct Monitor {
    sys: System,
    profile_gpus: Vec<GpuInfo>,
    /// Sensores/discos/interfaces são enumerados UMA vez aqui e apenas
    /// `refresh()` a cada amostra — recriar as listas a 1 Hz custaria uma
    /// nova enumeração do SO por segundo.
    components: Components,
    disks: Disks,
    networks: Networks,
    /// Pasta de dados do app: identifica QUAL disco reportar (o mount mais
    /// específico que a contém). `None` ⇒ cai no maior disco.
    data_dir: Option<PathBuf>,
    /// Totais acumulados de rede da última amostra + o instante REAL dela.
    /// O ticker do chamador pula ticks atrasados, então a taxa divide pelo
    /// intervalo medido — nunca por 1 s fixo.
    net_prev: NetSnapshot,
    /// Estado persistente de GPU no Windows (NVML + adaptadores DXGI + query
    /// PDH) — handles abertos uma vez e reutilizados a cada amostra.
    #[cfg(windows)]
    win_gpu: windows_gpu::WinGpuMonitor,
}

/// Leitura acumulada (desde o boot) de rx/tx somada em todas as interfaces,
/// com o instante em que foi tirada.
struct NetSnapshot {
    at: Instant,
    rx_total: u64,
    tx_total: u64,
}

impl Monitor {
    /// `data_dir`: pasta de dados do app, usada só para escolher o disco da
    /// telemetria. Opcional porque o Monitor também roda em testes/CLIs onde
    /// não há app — aí vale o maior disco (documentado em [`pick_disk`]).
    pub fn new(profile: &HardwareProfile, data_dir: Option<PathBuf>) -> Self {
        let mut sys = System::new();
        // Primeira leitura de CPU é descartável (diferencial).
        sys.refresh_cpu_usage();
        let networks = Networks::new_with_refreshed_list();
        let (rx_total, tx_total) = net_totals(&networks);
        Self {
            sys,
            profile_gpus: profile.gpus.clone(),
            components: Components::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list_specifics(disk_refresh_kind()),
            networks,
            data_dir,
            // Linha de base: a primeira amostra já sai com taxa real
            // (delta desde a criação, ~1 tick depois).
            net_prev: NetSnapshot {
                at: Instant::now(),
                rx_total,
                tx_total,
            },
            #[cfg(windows)]
            win_gpu: windows_gpu::WinGpuMonitor::new(&profile.gpus),
        }
    }

    pub fn sample(&mut self) -> Telemetry {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        let cpu_percent = self.sys.global_cpu_usage();
        let (gpus, gpu_temp_c) = self.sample_gpus();
        let (disk_used_pct, disk_free_bytes) = self.sample_disk();
        let (net_rx_bytes_per_sec, net_tx_bytes_per_sec) = self.sample_net();

        Telemetry {
            cpu_percent,
            ram_used_bytes: self.sys.used_memory(),
            ram_total_bytes: self.sys.total_memory(),
            gpus,
            cpu_temp_c: self.sample_cpu_temp(),
            gpu_temp_c,
            disk_used_pct,
            disk_free_bytes,
            net_rx_bytes_per_sec,
            net_tx_bytes_per_sec,
            ts_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    fn sample_gpus(&mut self) -> (Vec<GpuTelemetry>, Option<f32>) {
        #[cfg(windows)]
        {
            self.win_gpu.sample(&self.profile_gpus)
        }
        #[cfg(not(windows))]
        {
            // Fora do Windows ainda não há caminho de telemetria de GPU —
            // logo também não há temperatura (sem inventar valor).
            let gpus = self
                .profile_gpus
                .iter()
                .map(|g| GpuTelemetry {
                    util_percent: None,
                    vram_used_bytes: None,
                    vram_total_bytes: g.vram_total_bytes,
                })
                .collect();
            (gpus, None)
        }
    }

    /// Temperatura da CPU via `sysinfo::Components`. No Windows a lista vem
    /// VAZIA sem privilégio de administrador (WMI/ACPI) — isso é esperado e
    /// vira `None` em silêncio, sem log por amostra.
    fn sample_cpu_temp(&mut self) -> Option<f32> {
        self.components.refresh(true);
        let readings: Vec<(&str, f32)> = self
            .components
            .iter()
            .filter_map(|c| c.temperature().map(|t| (c.label(), t)))
            .collect();
        pick_cpu_temp(&readings)
    }

    /// % usado e bytes livres do disco onde mora a pasta de dados do app.
    fn sample_disk(&mut self) -> (Option<f32>, Option<u64>) {
        self.disks.refresh_specifics(true, disk_refresh_kind());
        let list: Vec<(&Path, u64, u64)> = self
            .disks
            .iter()
            .map(|d| (d.mount_point(), d.total_space(), d.available_space()))
            .collect();
        match pick_disk(self.data_dir.as_deref(), &list) {
            Some((total, avail)) if total > 0 => {
                let used_pct = ((total - avail) as f64 / total as f64 * 100.0) as f32;
                (Some(used_pct), Some(avail))
            }
            _ => (None, None),
        }
    }

    /// Taxa agregada de rede (todas as interfaces) desde a última amostra.
    fn sample_net(&mut self) -> (Option<u64>, Option<u64>) {
        self.networks.refresh(true);
        let now = Instant::now();
        let (rx_total, tx_total) = net_totals(&self.networks);
        let dt = now.duration_since(self.net_prev.at);
        // `saturating_sub`: uma interface que desaparece (VPN caiu) derruba o
        // total acumulado — melhor um zero pontual do que um estouro.
        let rx = bytes_per_sec(rx_total.saturating_sub(self.net_prev.rx_total), dt);
        let tx = bytes_per_sec(tx_total.saturating_sub(self.net_prev.tx_total), dt);
        self.net_prev = NetSnapshot {
            at: now,
            rx_total,
            tx_total,
        };
        (rx, tx)
    }
}

/// Soma dos acumulados de rx/tx de todas as interfaces (desde o boot).
fn net_totals(networks: &Networks) -> (u64, u64) {
    networks.iter().fold((0, 0), |(rx, tx), (_, data)| {
        (rx + data.total_received(), tx + data.total_transmitted())
    })
}

/// Delta de bytes ÷ intervalo realmente decorrido. `None` se o intervalo for
/// nulo (duas amostras no mesmo instante não medem taxa nenhuma).
fn bytes_per_sec(delta_bytes: u64, dt: Duration) -> Option<u64> {
    if dt.is_zero() {
        return None;
    }
    Some((delta_bytes as f64 / dt.as_secs_f64()).round() as u64)
}

/// Escolhe a temperatura "da CPU" entre os sensores disponíveis: componentes
/// com "cpu"/"package"/"tctl" no rótulo (Intel expõe "Package", AMD "Tctl");
/// havendo vários, a média deles. Sem nenhum rótulo reconhecível, a média de
/// todos — melhor um número aproximado do que descartar sensores genéricos.
fn pick_cpu_temp(readings: &[(&str, f32)]) -> Option<f32> {
    let cpuish: Vec<f32> = readings
        .iter()
        .filter(|(label, _)| {
            let l = label.to_ascii_lowercase();
            l.contains("cpu") || l.contains("package") || l.contains("tctl")
        })
        .map(|&(_, t)| t)
        .collect();
    if !cpuish.is_empty() {
        return Some(cpuish.iter().sum::<f32>() / cpuish.len() as f32);
    }
    if readings.is_empty() {
        return None;
    }
    Some(readings.iter().map(|&(_, t)| t).sum::<f32>() / readings.len() as f32)
}

/// Escolhe o disco a reportar: o de mount point MAIS ESPECÍFICO (mais longo)
/// que contém `data_dir` — em `/` + `/home`, a pasta em `/home/x` pertence ao
/// segundo. Sem `data_dir` (ou sem mount que a contenha), o maior disco: é
/// onde os modelos GGUF de dezenas de GiB tendem a morar.
fn pick_disk(data_dir: Option<&Path>, disks: &[(&Path, u64, u64)]) -> Option<(u64, u64)> {
    if let Some(dir) = data_dir {
        let best = disks
            .iter()
            .filter(|(mount, _, _)| dir.starts_with(mount))
            .max_by_key(|(mount, _, _)| mount.as_os_str().len());
        if let Some(&(_, total, avail)) = best {
            return Some((total, avail));
        }
    }
    disks
        .iter()
        .max_by_key(|&&(_, total, _)| total)
        .map(|&(_, total, avail)| (total, avail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_sane_profile() {
        let p = detect();
        assert!(p.cpu_cores > 0);
        assert!(p.ram_total_bytes > 1 << 28, "RAM deveria ser > 256 MiB");
        assert!(!p.cpu_name.is_empty());
    }

    #[test]
    fn monitor_samples() {
        let p = detect();
        let mut m = Monitor::new(&p, None);
        std::thread::sleep(std::time::Duration::from_millis(250));
        let t = m.sample();
        assert!(t.ram_used_bytes > 0);
        assert!(t.cpu_percent >= 0.0 && t.cpu_percent <= 100.0);
    }

    /// Contrato com o frontend: os campos novos existem no JSON, em camelCase,
    /// e valem `null` quando a plataforma não fornece a métrica.
    #[test]
    fn the_payload_serializes_the_new_fields_as_camel_case_nulls() {
        let t = Telemetry {
            cpu_percent: 12.5,
            ram_used_bytes: 1,
            ram_total_bytes: 2,
            gpus: Vec::new(),
            cpu_temp_c: None,
            gpu_temp_c: None,
            disk_used_pct: None,
            disk_free_bytes: None,
            net_rx_bytes_per_sec: None,
            net_tx_bytes_per_sec: None,
            ts_ms: 0,
        };
        let json = serde_json::to_value(&t).expect("Telemetry sempre serializa");
        for key in [
            "cpuTempC",
            "gpuTempC",
            "diskUsedPct",
            "diskFreeBytes",
            "netRxBytesPerSec",
            "netTxBytesPerSec",
        ] {
            assert!(
                json.get(key).is_some_and(|v| v.is_null()),
                "campo {key} deveria estar presente e nulo em {json}"
            );
        }
    }

    #[test]
    fn the_network_rate_divides_by_the_measured_interval_not_a_fixed_second() {
        // 3000 bytes em 2 s = 1500 B/s — se alguém trocar por "÷ 1 s fixo",
        // este teste quebra.
        assert_eq!(
            bytes_per_sec(3000, std::time::Duration::from_secs(2)),
            Some(1500)
        );
        // Tick atrasado: 1000 bytes em 250 ms = 4000 B/s.
        assert_eq!(
            bytes_per_sec(1000, std::time::Duration::from_millis(250)),
            Some(4000)
        );
    }

    #[test]
    fn a_zero_interval_yields_no_rate_instead_of_dividing_by_zero() {
        assert_eq!(bytes_per_sec(1234, std::time::Duration::ZERO), None);
    }

    #[test]
    fn cpu_temp_prefers_sensors_that_look_like_the_cpu() {
        // A GPU a 80 °C não pode contaminar a leitura quando existe "Package".
        let readings = [("nvme composite", 40.0), ("Package id 0", 55.0)];
        assert_eq!(pick_cpu_temp(&readings), Some(55.0));

        // AMD expõe "Tctl".
        let readings = [("Tctl", 60.0), ("gpu edge", 80.0)];
        assert_eq!(pick_cpu_temp(&readings), Some(60.0));
    }

    #[test]
    fn cpu_temp_falls_back_to_the_average_when_no_label_matches() {
        let readings = [("sensor a", 40.0), ("sensor b", 60.0)];
        assert_eq!(pick_cpu_temp(&readings), Some(50.0));
        assert_eq!(pick_cpu_temp(&[]), None);
    }

    #[test]
    fn the_disk_report_picks_the_most_specific_mount_containing_the_data_dir() {
        let root = Path::new("/");
        let home = Path::new("/home");
        let disks = [(root, 100, 10), (home, 200, 20)];
        // `/home/x` pertence a `/home`, não a `/` — o mount mais longo vence.
        assert_eq!(
            pick_disk(Some(Path::new("/home/x/dados")), &disks),
            Some((200, 20))
        );
        // Sem data_dir, vale o maior disco.
        assert_eq!(pick_disk(None, &disks), Some((200, 20)));
        assert_eq!(pick_disk(None, &[]), None);
    }
}
