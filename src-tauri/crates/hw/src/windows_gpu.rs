//! Detecção e telemetria de GPU no Windows (qualquer vendor).
//!
//! Fontes:
//! - **Detecção**: DXGI `EnumAdapters1` + `DXGI_ADAPTER_DESC1` enumera todos os
//!   adaptadores de hardware (WARP/`DXGI_ADAPTER_FLAG_SOFTWARE` é filtrado).
//!   Para NVIDIA, os dados do NVML (nome/driver/compute capability/VRAM) têm
//!   prioridade — os adaptadores DXGI são deduplicados contra os devices NVML
//!   por nome e, em último caso, pela ordem de enumeração.
//! - **iGPU vs dGPU**: device D3D12 mínimo no adaptador (feature level 11.0) +
//!   `CheckFeatureSupport(D3D12_FEATURE_ARCHITECTURE1)`: `UMA == true` ⇒
//!   integrada. NUNCA usar apenas `DedicatedVideoMemory > 0` — iGPUs Intel
//!   reportam valor ≠ 0 (bug conhecido, wgpu#683). Se o D3D12 falhar, cai na
//!   heurística documentada em [`is_integrated`].
//! - **VRAM usada** (qualquer vendor): `IDXGIAdapter3::QueryVideoMemoryInfo`
//!   no segmento LOCAL (`CurrentUsage`).
//! - **Utilização NVIDIA**: NVML `utilization_rates`.
//! - **Utilização AMD/Intel**: contadores PDH `\GPU Engine(*)\Utilization
//!   Percentage` — soma por (LUID, engine type) e MÁXIMO entre engine types
//!   por LUID, como o Task Manager. A query PDH é persistente no
//!   [`WinGpuMonitor`] (aberta uma vez, fechada no `Drop`).
//!
//! Qualquer falha vira campo `None` na telemetria (nunca panic; `log::debug`).

use std::collections::HashMap;

use lr_types::{GpuInfo, GpuTelemetry, GpuVendor};
use nvml_wrapper::Nvml;

/// O NVML fala em miliwatts; a tela fala em watts.
fn mw_para_w(mw: u32) -> u32 {
    mw.div_ceil(1000)
}
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_FEATURE_ARCHITECTURE1, D3D12_FEATURE_DATA_ARCHITECTURE1, D3D12CreateDevice, ID3D12Device,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
    DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter1, IDXGIAdapter3,
    IDXGIFactory1,
};
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::core::{Interface, PCWSTR, w};

/// PCI vendor IDs relevantes para inferência local.
const VENDOR_ID_NVIDIA: u32 = 0x10DE;
const VENDOR_ID_AMD: u32 = 0x1002;
const VENDOR_ID_INTEL: u32 = 0x8086;

// ---------------------------------------------------------------------------
// Detecção
// ---------------------------------------------------------------------------

pub fn detect() -> Vec<GpuInfo> {
    let nvml_gpus = detect_nvml();
    match detect_dxgi(&nvml_gpus) {
        Ok(gpus) if !gpus.is_empty() => gpus,
        Ok(_) => {
            log::debug!("DXGI não enumerou adaptadores de hardware; usando apenas NVML");
            nvml_gpus
        }
        Err(e) => {
            log::debug!("enumeração DXGI falhou ({e}); usando apenas NVML");
            nvml_gpus
        }
    }
}
/// Banda de memória da placa, em bytes por segundo.
///
/// `relógio de memória × 2 × largura do barramento / 8` — o ×2 é o "double
/// data rate" das memórias GDDR, e é o que faz a conta bater com a ficha
/// técnica: uma RTX 3090 reporta 9751 MHz em 384 bits, que dão os 936 GB/s
/// anunciados. Qualquer dos dois números faltando devolve `None`: metade da
/// conta não é uma banda.
fn banda_nvml(dev: &nvml_wrapper::Device<'_>) -> Option<u64> {
    use nvml_wrapper::enum_wrappers::device::Clock;
    let mhz = dev.max_clock_info(Clock::Memory).ok()? as u64;
    let bits = dev.memory_bus_width().ok()? as u64;
    (mhz > 0 && bits > 0).then(|| mhz * 1_000_000 * 2 * bits / 8)
}

/// Devices NVIDIA via NVML: nome, driver, compute capability e VRAM confiáveis.
fn detect_nvml() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if let Ok(nvml) = Nvml::init() {
        let driver = nvml.sys_driver_version().ok();
        let count = nvml.device_count().unwrap_or(0);
        for i in 0..count {
            let Ok(dev) = nvml.device_by_index(i) else {
                continue;
            };
            let name = dev.name().unwrap_or_else(|_| "GPU NVIDIA".to_string());
            let vram = dev.memory_info().map(|m| m.total).unwrap_or(0);
            let cc = dev
                .cuda_compute_capability()
                .ok()
                .map(|c| (c.major as u32, c.minor as u32));
            gpus.push(GpuInfo {
                name,
                vendor: GpuVendor::Nvidia,
                vram_total_bytes: vram,
                is_integrated: false,
                driver_version: driver.clone(),
                cuda_compute: cc,
                bandwidth_bytes_s: banda_nvml(&dev),
            });
        }
    }
    gpus
}

/// Enumera todos os adaptadores de hardware via DXGI, mesclando os dados NVML
/// nos adaptadores NVIDIA (dedup por nome, senão pela ordem de enumeração).
fn detect_dxgi(nvml_gpus: &[GpuInfo]) -> windows::core::Result<Vec<GpuInfo>> {
    // SAFETY: chamadas COM com os wrappers do crate `windows`; as interfaces
    // são liberadas via `Drop` (Release automático).
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut out = Vec::new();
        let mut nvml_used = vec![false; nvml_gpus.len()];

        let mut index = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e),
            };
            index += 1;

            let desc = match adapter.GetDesc1() {
                Ok(d) => d,
                Err(e) => {
                    log::debug!("GetDesc1 falhou no adaptador {}: {e}", index - 1);
                    continue;
                }
            };
            // Pula WARP e outros rasterizadores por software.
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }

            let name = utf16_to_string(&desc.Description);
            let vendor = vendor_from_id(desc.VendorId);
            let integrated = is_integrated(&adapter, &desc);

            if vendor == GpuVendor::Nvidia {
                // Dedup contra NVML: por nome; senão o próximo ainda não usado.
                let matched = nvml_gpus
                    .iter()
                    .enumerate()
                    .position(|(j, g)| !nvml_used[j] && g.name.eq_ignore_ascii_case(&name))
                    .or_else(|| nvml_used.iter().position(|used| !used));
                if let Some(j) = matched {
                    nvml_used[j] = true;
                    let mut gpu = nvml_gpus[j].clone();
                    gpu.is_integrated = integrated;
                    out.push(gpu);
                    continue;
                }
                // NVML indisponível: segue com os dados do DXGI mesmo.
            }

            out.push(GpuInfo {
                name,
                vendor,
                vram_total_bytes: desc.DedicatedVideoMemory as u64,
                is_integrated: integrated,
                driver_version: None,
                cuda_compute: None,
                // O DXGI não expõe relógio nem largura do barramento; sem
                // NVML, a banda fica sem resposta em vez de virar chute.
                bandwidth_bytes_s: None,
            });
        }

        // Devices NVML que o DXGI não enxergou (raro) não podem sumir do perfil.
        for (j, gpu) in nvml_gpus.iter().enumerate() {
            if !nvml_used[j] {
                out.push(gpu.clone());
            }
        }
        Ok(out)
    }
}

fn vendor_from_id(vendor_id: u32) -> GpuVendor {
    match vendor_id {
        VENDOR_ID_NVIDIA => GpuVendor::Nvidia,
        VENDOR_ID_AMD => GpuVendor::Amd,
        VENDOR_ID_INTEL => GpuVendor::Intel,
        _ => GpuVendor::Other,
    }
}

/// Converte a `Description` (UTF-16 com terminador NUL) em `String`.
fn utf16_to_string(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len]).trim().to_string()
}

/// Classifica o adaptador como integrado (UMA) ou dedicado.
///
/// Caminho preferido: device D3D12 mínimo + `D3D12_FEATURE_ARCHITECTURE1`
/// (campo `UMA`). Se o D3D12 não estiver disponível (driver antigo, feature
/// level < 11.0), usa a heurística documentada: VRAM dedicada < 1 GiB E
/// memória compartilhada maior que a dedicada ⇒ integrada. Só o valor de
/// `DedicatedVideoMemory` não basta — iGPUs Intel reportam ≠ 0 (wgpu#683).
fn is_integrated(adapter: &IDXGIAdapter1, desc: &DXGI_ADAPTER_DESC1) -> bool {
    match uma_via_d3d12(adapter) {
        Some(uma) => uma,
        None => {
            const GIB: usize = 1 << 30;
            desc.DedicatedVideoMemory < GIB && desc.SharedSystemMemory > desc.DedicatedVideoMemory
        }
    }
}

/// `UMA` via `CheckFeatureSupport(D3D12_FEATURE_ARCHITECTURE1)`.
/// `None` quando o adaptador não suporta D3D12 (feature level 11.0).
fn uma_via_d3d12(adapter: &IDXGIAdapter1) -> Option<bool> {
    // SAFETY: device D3D12 temporário só para a consulta; liberado no fim do
    // escopo via `Drop`. A struct de saída tem exatamente o tamanho declarado.
    unsafe {
        let mut device: Option<ID3D12Device> = None;
        if let Err(e) = D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_11_0, &mut device) {
            log::debug!("D3D12CreateDevice falhou na classificação iGPU/dGPU: {e}");
            return None;
        }
        let device = device?;
        let mut arch = D3D12_FEATURE_DATA_ARCHITECTURE1::default();
        if let Err(e) = device.CheckFeatureSupport(
            D3D12_FEATURE_ARCHITECTURE1,
            &mut arch as *mut _ as *mut core::ffi::c_void,
            size_of::<D3D12_FEATURE_DATA_ARCHITECTURE1>() as u32,
        ) {
            log::debug!("CheckFeatureSupport(ARCHITECTURE1) falhou: {e}");
            return None;
        }
        Some(arch.UMA.as_bool())
    }
}

// ---------------------------------------------------------------------------
// Telemetria
// ---------------------------------------------------------------------------

/// LUID no formato usado pelas instâncias PDH: (HighPart, LowPart), ambos
/// impressos como `0x%08X` — ver [`parse_engine_instance`].
type PdhLuid = (u32, u32);

/// Estado persistente de telemetria de GPU no Windows. Uma instância por
/// `Monitor`; reutiliza handles (NVML, adaptadores DXGI, query PDH) entre
/// amostras para não abrir/alocar nada no loop de 1 Hz.
pub struct WinGpuMonitor {
    nvml: Option<Nvml>,
    /// Um slot por GPU do profile, NA MESMA ORDEM (contrato com a UI):
    /// adaptador DXGI casado por vendor+nome (fallback: ordem por vendor).
    slots: Vec<Option<AdapterSlot>>,
    /// Índice NVML de cada GPU do profile (None para não-NVIDIA). Resolvido
    /// por nome em `new()` — a ordem de enumeração do NVML NÃO é a do DXGI,
    /// então um contador posicional atribuiria telemetria à GPU errada em
    /// máquinas com várias NVIDIA.
    nvml_indices: Vec<Option<u32>>,
    /// Query PDH de utilização de GPU; só aberta se houver GPU não-NVIDIA.
    pdh: Option<PdhGpuUtilQuery>,
}

struct AdapterSlot {
    adapter: IDXGIAdapter3,
    luid: PdhLuid,
}

impl WinGpuMonitor {
    pub fn new(profile_gpus: &[GpuInfo]) -> Self {
        let slots = match match_adapters(profile_gpus) {
            Ok(slots) => slots,
            Err(e) => {
                log::debug!("mapeamento DXGI para telemetria falhou: {e}");
                profile_gpus.iter().map(|_| None).collect()
            }
        };
        // NVIDIA usa NVML para utilização; PDH cobre AMD/Intel/outros.
        let pdh = if profile_gpus.iter().any(|g| g.vendor != GpuVendor::Nvidia) {
            match PdhGpuUtilQuery::open() {
                Ok(q) => Some(q),
                Err(status) => {
                    log::debug!("query PDH de GPU indisponível (status 0x{status:08X})");
                    None
                }
            }
        } else {
            None
        };
        let nvml = Nvml::init().ok();
        let nvml_indices = match_nvml_indices(nvml.as_ref(), profile_gpus);
        Self {
            nvml,
            slots,
            nvml_indices,
            pdh,
        }
    }

    /// Telemetria por GPU (na mesma ordem de `profile_gpus`) + temperatura em
    /// °C da primeira GPU que reportar uma. Hoje só o NVML expõe temperatura;
    /// o PDH (AMD/Intel) não tem contador de temperatura — fica `None` sem
    /// inventar valor.
    pub fn sample(&mut self, profile_gpus: &[GpuInfo]) -> (Vec<GpuTelemetry>, Option<f32>) {
        // Uma coleta PDH por amostra: o valor formatado é a taxa desde a
        // coleta anterior (o Monitor roda a ~1 Hz, intervalo suficiente).
        let pdh_util = self.pdh.as_mut().and_then(PdhGpuUtilQuery::collect);

        let mut temp_c: Option<f32> = None;
        let mut out = Vec::with_capacity(profile_gpus.len());
        for (i, gpu) in profile_gpus.iter().enumerate() {
            let slot = self.slots.get(i).and_then(Option::as_ref);

            // VRAM usada no segmento local do adaptador (qualquer vendor).
            let mut vram_used = slot.and_then(|s| query_local_vram_used(&s.adapter));
            let mut vram_total = gpu.vram_total_bytes;
            let mut util = None;
            let mut power_w = None;
            let mut power_limit_w = None;

            if gpu.vendor == GpuVendor::Nvidia {
                if let Some(nvml) = self.nvml.as_ref()
                    && let Some(Some(idx)) = self.nvml_indices.get(i)
                    && let Ok(dev) = nvml.device_by_index(*idx)
                {
                    util = dev.utilization_rates().ok().map(|u| u.gpu as f32);
                    // NVML é mais preciso que o DXGI para VRAM de NVIDIA.
                    if let Ok(mem) = dev.memory_info() {
                        vram_used = Some(mem.used);
                        vram_total = mem.total;
                    }
                    // O payload tem UM campo de temperatura: vale a primeira
                    // GPU com leitura (na ordem do profile — a dedicada que o
                    // usuário usa para inferência vem primeiro).
                    if temp_c.is_none() {
                        temp_c = dev
                            .temperature(TemperatureSensor::Gpu)
                            .ok()
                            .map(|t| t as f32);
                    }
                    // Watts na mesma passada: o NVML já está aberto e o
                    // device já está em mãos.
                    power_w = dev.power_usage().ok().map(mw_para_w);
                    power_limit_w = dev.power_management_limit().ok().map(mw_para_w);
                }
            } else if let (Some(map), Some(slot)) = (pdh_util.as_ref(), slot) {
                util = map.get(&slot.luid).copied();
            }

            out.push(GpuTelemetry {
                util_percent: util,
                vram_used_bytes: vram_used,
                vram_total_bytes: vram_total,
                power_w,
                power_limit_w,
            });
        }
        (out, temp_c)
    }
}

/// Casa cada GPU NVIDIA do profile com um índice de device NVML, por nome
/// (ignore-case) com fallback para o próximo device livre — espelha o dedup
/// de `detect_dxgi`, garantindo que a telemetria leia o MESMO device cujos
/// dados foram fundidos no profile.
fn match_nvml_indices(nvml: Option<&Nvml>, profile_gpus: &[GpuInfo]) -> Vec<Option<u32>> {
    let mut devices: Vec<(u32, String, bool)> = Vec::new();
    if let Some(nvml) = nvml {
        let count = nvml.device_count().unwrap_or(0);
        for i in 0..count {
            if let Ok(dev) = nvml.device_by_index(i) {
                devices.push((i, dev.name().unwrap_or_default(), false));
            }
        }
    }
    profile_gpus
        .iter()
        .map(|gpu| {
            if gpu.vendor != GpuVendor::Nvidia {
                return None;
            }
            let by_name = devices
                .iter()
                .position(|(_, name, used)| !used && name.eq_ignore_ascii_case(&gpu.name));
            let pick = by_name.or_else(|| devices.iter().position(|(_, _, used)| !used))?;
            devices[pick].2 = true;
            Some(devices[pick].0)
        })
        .collect()
}

/// Casa cada GPU do profile com um adaptador DXGI (por vendor+nome; fallback
/// para o próximo adaptador livre do mesmo vendor). Mantém a ordem do profile.
fn match_adapters(profile_gpus: &[GpuInfo]) -> windows::core::Result<Vec<Option<AdapterSlot>>> {
    struct Candidate {
        name: String,
        vendor: GpuVendor,
        luid: PdhLuid,
        adapter: IDXGIAdapter3,
        used: bool,
    }

    // SAFETY: mesmas garantias de `detect_dxgi` (COM via wrappers com Drop).
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut candidates: Vec<Candidate> = Vec::new();

        let mut index = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => break,
            };
            index += 1;

            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            // IDXGIAdapter3 (QueryVideoMemoryInfo) existe desde o Windows 10.
            let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>() else {
                log::debug!("adaptador sem IDXGIAdapter3; sem VRAM usada para ele");
                continue;
            };
            candidates.push(Candidate {
                name: utf16_to_string(&desc.Description),
                vendor: vendor_from_id(desc.VendorId),
                luid: luid_key(desc.AdapterLuid),
                adapter: adapter3,
                used: false,
            });
        }

        let mut slots = Vec::with_capacity(profile_gpus.len());
        for gpu in profile_gpus {
            let found = candidates
                .iter()
                .position(|c| {
                    !c.used && c.vendor == gpu.vendor && c.name.eq_ignore_ascii_case(&gpu.name)
                })
                .or_else(|| {
                    candidates
                        .iter()
                        .position(|c| !c.used && c.vendor == gpu.vendor)
                });
            slots.push(found.map(|p| {
                candidates[p].used = true;
                AdapterSlot {
                    adapter: candidates[p].adapter.clone(),
                    luid: candidates[p].luid,
                }
            }));
        }
        Ok(slots)
    }
}

/// Chave (HighPart, LowPart) na forma impressa pelas instâncias PDH.
fn luid_key(luid: LUID) -> PdhLuid {
    (luid.HighPart as u32, luid.LowPart)
}

/// `CurrentUsage` do segmento de memória LOCAL (VRAM física do adaptador).
fn query_local_vram_used(adapter: &IDXGIAdapter3) -> Option<u64> {
    // SAFETY: struct de saída zero-inicializada com o layout esperado pela API.
    unsafe {
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        match adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) {
            Ok(()) => Some(info.CurrentUsage),
            Err(e) => {
                log::debug!("QueryVideoMemoryInfo falhou: {e}");
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PDH: \GPU Engine(*)\Utilization Percentage
// ---------------------------------------------------------------------------

/// Query PDH persistente para utilização de GPU (AMD/Intel). Aberta uma vez
/// no [`WinGpuMonitor::new`]; o handle é fechado no [`Drop`].
struct PdhGpuUtilQuery {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    /// Buffer reutilizado entre amostras para o array formatado do PDH.
    /// `u64` garante alinhamento de 8 bytes (itens contêm ponteiros) e evita
    /// realocação no loop de 1 Hz — só cresce quando o PDH pede mais espaço.
    buf: Vec<u64>,
}

// SAFETY: handles PDH são identificadores opacos e a API PDH é thread-safe;
// a query só é usada por uma thread por vez (métodos exigem `&mut self`).
unsafe impl Send for PdhGpuUtilQuery {}

impl PdhGpuUtilQuery {
    /// Caminho em inglês (via `PdhAddEnglishCounterW`) — independe do idioma
    /// do Windows. Instâncias no formato
    /// `pid_1234_luid_0x00000000_0x0000C122_phys_0_engtype_3D`.
    const COUNTER_PATH: PCWSTR = w!("\\GPU Engine(*)\\Utilization Percentage");

    /// Abre a query e faz a primeira coleta (contadores de taxa precisam de
    /// duas coletas; a segunda acontece no primeiro `collect`).
    fn open() -> Result<Self, u32> {
        // SAFETY: handles de saída zero-inicializados; em erro parcial a query
        // é fechada aqui mesmo para não vazar.
        unsafe {
            let mut query = PDH_HQUERY::default();
            let status = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
            if status != 0 {
                return Err(status);
            }
            let mut counter = PDH_HCOUNTER::default();
            let status = PdhAddEnglishCounterW(query, Self::COUNTER_PATH, 0, &mut counter);
            if status != 0 {
                PdhCloseQuery(query);
                return Err(status);
            }
            let status = PdhCollectQueryData(query);
            if status != 0 {
                log::debug!("coleta PDH inicial falhou (status 0x{status:08X})");
            }
            Ok(Self {
                query,
                counter,
                buf: Vec::new(),
            })
        }
    }

    /// Coleta uma amostra e devolve a utilização (0–100) por LUID: soma por
    /// (LUID, engine type), depois MÁXIMO entre engine types — o mesmo número
    /// que o Task Manager mostra por GPU.
    fn collect(&mut self) -> Option<HashMap<PdhLuid, f32>> {
        // SAFETY: buffer com alinhamento/tamanho válidos para os itens; o PDH
        // escreve `count` itens e as strings apontam para dentro do buffer.
        unsafe {
            let status = PdhCollectQueryData(self.query);
            if status != 0 {
                log::debug!("PdhCollectQueryData falhou (status 0x{status:08X})");
                return None;
            }
            // Até 2 tentativas: instâncias surgem/somem (processos), então a
            // 1ª chamada pode devolver PDH_MORE_DATA com o tamanho necessário.
            for _ in 0..2 {
                let mut size = (self.buf.len() * size_of::<u64>()) as u32;
                let mut count = 0u32;
                let items = (!self.buf.is_empty())
                    .then_some(self.buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W);
                let status = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    items,
                );
                if status == PDH_MORE_DATA {
                    self.buf
                        .resize((size as usize).div_ceil(size_of::<u64>()), 0);
                    continue;
                }
                if status != 0 {
                    log::debug!("PdhGetFormattedCounterArrayW falhou (status 0x{status:08X})");
                    return None;
                }
                return Some(self.aggregate(count));
            }
            None
        }
    }

    /// Agrega os `count` itens já formatados no buffer.
    fn aggregate(&self, count: u32) -> HashMap<PdhLuid, f32> {
        let mut per_engine: HashMap<(PdhLuid, String), f64> = HashMap::new();
        // SAFETY: `count` itens válidos escritos pelo PDH no início do buffer.
        unsafe {
            let items = std::slice::from_raw_parts(
                self.buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W,
                count as usize,
            );
            for item in items {
                let cstatus = item.FmtValue.CStatus;
                if cstatus != PDH_CSTATUS_VALID_DATA && cstatus != PDH_CSTATUS_NEW_DATA {
                    continue;
                }
                let Ok(name) = item.szName.to_string() else {
                    continue;
                };
                let Some((luid, engtype)) = parse_engine_instance(&name) else {
                    continue;
                };
                *per_engine.entry((luid, engtype)).or_insert(0.0) +=
                    item.FmtValue.Anonymous.doubleValue;
            }
        }
        // Máximo entre engine types por LUID (semântica do Task Manager).
        let mut per_luid: HashMap<PdhLuid, f32> = HashMap::new();
        for ((luid, _engtype), total) in per_engine {
            let value = total.clamp(0.0, 100.0) as f32;
            let entry = per_luid.entry(luid).or_insert(0.0);
            if value > *entry {
                *entry = value;
            }
        }
        per_luid
    }
}

impl Drop for PdhGpuUtilQuery {
    fn drop(&mut self) {
        // SAFETY: handle aberto em `open`, fechado exatamente uma vez aqui.
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

/// Extrai (LUID, engine type) de um nome de instância PDH no formato
/// `pid_1234_luid_0xAAAAAAAA_0xBBBBBBBB_phys_0_engtype_3D`
/// (primeiro hex = HighPart, segundo = LowPart).
fn parse_engine_instance(name: &str) -> Option<(PdhLuid, String)> {
    let after_luid = name.split_once("luid_")?.1;
    let mut parts = after_luid.split('_');
    let high = parse_hex_u32(parts.next()?)?;
    let low = parse_hex_u32(parts.next()?)?;
    let engtype = name.rsplit_once("engtype_")?.1.to_string();
    Some(((high, low), engtype))
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_engine_instance_extracts_luid_and_engtype() {
        let ((high, low), engtype) =
            parse_engine_instance("pid_9988_luid_0x00000000_0x0000C122_phys_0_engtype_3D")
                .expect("instância válida");
        assert_eq!(high, 0);
        assert_eq!(low, 0xC122);
        assert_eq!(engtype, "3D");
    }

    #[test]
    fn parse_engine_instance_keeps_compound_engtype() {
        let (_, engtype) =
            parse_engine_instance("pid_4_luid_0x00000000_0x0000D3A8_phys_0_engtype_Video Decode")
                .expect("instância válida");
        assert_eq!(engtype, "Video Decode");
    }

    #[test]
    fn parse_engine_instance_rejects_garbage() {
        assert!(parse_engine_instance("pid_1_phys_0_engtype_3D").is_none());
        assert!(parse_engine_instance("luid_zzz_0x1_engtype_3D").is_none());
        assert!(parse_engine_instance("").is_none());
    }

    #[test]
    fn utf16_to_string_stops_at_nul() {
        let mut wide = [0u16; 128];
        for (i, c) in "Radeon RX 7800 XT".encode_utf16().enumerate() {
            wide[i] = c;
        }
        assert_eq!(utf16_to_string(&wide), "Radeon RX 7800 XT");
    }
}
