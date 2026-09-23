//! Detecção e telemetria de GPU no Linux (qualquer marca).
//!
//! Fontes:
//! - **Enumeração**: o DRM no sysfs — cada `/sys/class/drm/cardN` cujo
//!   `device` é uma controladora de vídeo PCI (classe `0x03xxxx`). É o que o
//!   kernel sabe de toda placa com driver carregado, seja qual for a marca.
//! - **NVIDIA**: NVML (`libnvidia-ml.so.1`, instalado pelo driver
//!   proprietário) — nome, driver, compute capability, VRAM e banda, como no
//!   Windows. O casamento com o sysfs é pelo endereço PCI, que é exato; sem
//!   NVML (nouveau, driver quebrado) a placa aparece com o que o sysfs diz.
//! - **AMD** (amdgpu): `mem_info_vram_total`/`mem_info_vram_used`,
//!   `gpu_busy_percent` e os sensores `hwmon` (temperatura e watts).
//! - **Intel** (i915/xe): o kernel não expõe VRAM nem utilização num arquivo
//!   simples. A placa aparece; o que ele não diz fica `None`/zero.
//! - **Nome** de quem não tem NVML: `product_name` quando o driver expõe,
//!   senão a base `pci.ids` do sistema (`hwdata`/`pciutils`).
//!
//! Qualquer falha vira campo `None` na telemetria (nunca panic).

use std::path::{Path, PathBuf};

use lr_types::{GpuInfo, GpuTelemetry, GpuVendor};
use nvml_wrapper::Nvml;

const DRM: &str = "/sys/class/drm";

/// Onde as distribuições guardam a base de nomes PCI (Fedora/Arch no
/// `hwdata`, Debian/Ubuntu no `pciutils`).
const PCI_IDS: [&str; 3] = [
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
];

const VENDOR_ID_NVIDIA: u32 = 0x10DE;
const VENDOR_ID_AMD: u32 = 0x1002;
const VENDOR_ID_INTEL: u32 = 0x8086;

const GIB: u64 = 1 << 30;

// ---------------------------------------------------------------------------
// Enumeração
// ---------------------------------------------------------------------------

/// Uma placa como o sysfs a vê.
#[derive(Debug, Clone)]
struct CartaoDrm {
    /// `/sys/class/drm/cardN/device` — de onde saem as leituras.
    device: PathBuf,
    /// Endereço PCI (`0000:05:00.0`), a chave que casa com o NVML.
    pci: String,
    vendor_id: u32,
    device_id: u32,
}

/// Os cartões DRM que são placas de vídeo PCI, na ordem `card0`, `card1`...
///
/// Ficam de fora os conectores (`card0-DP-1`), os nós de render
/// (`renderD128`) e os dispositivos sem PCI (o `simpledrm` do framebuffer de
/// boot), que não têm `vendor`.
fn cartoes(raiz: &Path) -> Vec<CartaoDrm> {
    let Ok(entradas) = std::fs::read_dir(raiz) else {
        return Vec::new();
    };
    let mut achados: Vec<(u32, CartaoDrm)> = Vec::new();
    for entrada in entradas.flatten() {
        let nome = entrada.file_name().to_string_lossy().into_owned();
        let Some(n) = nome
            .strip_prefix("card")
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let device = entrada.path().join("device");
        let (Some(vendor_id), Some(device_id)) = (
            ler_hex(&device.join("vendor")),
            ler_hex(&device.join("device")),
        ) else {
            continue;
        };
        if ler_hex(&device.join("class")).is_some_and(|classe| classe >> 16 != 0x03) {
            continue;
        }
        // O link `device` aponta para o dispositivo PCI; o último componente
        // do caminho real é o endereço.
        let pci = std::fs::canonicalize(&device)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        if !pci.is_empty() && achados.iter().any(|(_, c)| c.pci == pci) {
            continue;
        }
        achados.push((
            n,
            CartaoDrm {
                device,
                pci,
                vendor_id,
                device_id,
            },
        ));
    }
    achados.sort_by_key(|(n, _)| *n);
    achados.into_iter().map(|(_, c)| c).collect()
}

/// Uma placa detectada, com o caminho de onde a telemetria vai lê-la.
#[derive(Debug, Clone)]
struct Placa {
    info: GpuInfo,
    nvml_index: Option<u32>,
    device: Option<PathBuf>,
}

/// Junta o que o sysfs e o NVML sabem, uma entrada por placa.
///
/// A NVIDIA com NVML leva os números do NVML (os mesmos do Windows); as
/// demais levam o que o sysfs diz. Placas que o NVML enxerga e o sysfs não
/// (um contêiner sem `/sys/class/drm`) entram no fim — não podem sumir do
/// perfil.
fn juntar(
    cartoes: Vec<CartaoDrm>,
    mut nvml: Vec<crate::nvml::Placa>,
    nome_pci: &mut dyn FnMut(u32, u32) -> Option<String>,
) -> Vec<Placa> {
    let mut out = Vec::new();
    for c in cartoes {
        if c.vendor_id == VENDOR_ID_NVIDIA
            && let Some(i) = nvml.iter().position(|p| p.pci.as_deref() == Some(&c.pci))
        {
            let p = nvml.remove(i);
            out.push(Placa {
                info: p.info,
                nvml_index: Some(p.index),
                device: Some(c.device),
            });
            continue;
        }
        out.push(Placa {
            info: info_do_sysfs(&c, nome_pci),
            nvml_index: None,
            device: Some(c.device),
        });
    }
    out.extend(nvml.into_iter().map(|p| Placa {
        info: p.info,
        nvml_index: Some(p.index),
        device: None,
    }));
    out
}

/// O que dá para saber de uma placa só pelo sysfs.
fn info_do_sysfs(c: &CartaoDrm, nome_pci: &mut dyn FnMut(u32, u32) -> Option<String>) -> GpuInfo {
    let vendor = match c.vendor_id {
        VENDOR_ID_NVIDIA => GpuVendor::Nvidia,
        VENDOR_ID_AMD => GpuVendor::Amd,
        VENDOR_ID_INTEL => GpuVendor::Intel,
        _ => GpuVendor::Other,
    };
    let name = ler_texto(&c.device.join("product_name"))
        .or_else(|| nome_pci(c.vendor_id, c.device_id).map(|bruto| nome_amigavel(vendor, &bruto)))
        .unwrap_or_else(|| {
            format!(
                "GPU {} ({:04x}:{:04x})",
                marca(vendor).unwrap_or("PCI"),
                c.vendor_id,
                c.device_id
            )
        });
    // Só o amdgpu expõe a VRAM aqui; i915/xe e o driver da NVIDIA não.
    let vram = ler_u64(&c.device.join("mem_info_vram_total")).unwrap_or(0);
    GpuInfo {
        name,
        vendor,
        vram_total_bytes: vram,
        is_integrated: integrada(vendor, vram),
        // Sem NVML, a versão do módulo da NVIDIA ainda diz qual driver roda.
        driver_version: if vendor == GpuVendor::Nvidia {
            ler_texto(Path::new("/sys/module/nvidia/version"))
        } else {
            None
        },
        cuda_compute: None,
        bandwidth_bytes_s: None,
    }
}

/// Integrada ou dedicada, só com o que o sysfs diz.
///
/// O kernel não tem um "é APU" num arquivo. O que ele tem é a memória
/// reservada: a de uma integrada AMD é o pedaço da RAM que o firmware separa
/// (512 MiB por padrão, raramente mais que 2 GiB), e a de uma Intel nem é
/// informada. Placa dedicada com menos de 2 GiB não serve para modelo de
/// linguagem de qualquer jeito. Errar aqui custa pouco: o motor é o mesmo
/// (Vulkan) e o campo só pesa na escolha da placa principal.
fn integrada(vendor: GpuVendor, vram: u64) -> bool {
    match vendor {
        GpuVendor::Nvidia => false,
        GpuVendor::Amd | GpuVendor::Intel | GpuVendor::Other => vram < 2 * GIB,
        GpuVendor::Apple => true,
    }
}

fn marca(vendor: GpuVendor) -> Option<&'static str> {
    match vendor {
        GpuVendor::Nvidia => Some("NVIDIA"),
        GpuVendor::Amd => Some("AMD"),
        GpuVendor::Intel => Some("Intel"),
        GpuVendor::Apple | GpuVendor::Other => None,
    }
}

/// `GA102 [GeForce RTX 3090]` → `NVIDIA GeForce RTX 3090`.
///
/// O `pci.ids` põe o nome comercial entre colchetes, depois do codinome do
/// chip. É o comercial que a pessoa reconhece — o mesmo que o Windows mostra.
fn nome_amigavel(vendor: GpuVendor, bruto: &str) -> String {
    let modelo = match (bruto.find('['), bruto.rfind(']')) {
        (Some(a), Some(b)) if b > a + 1 => &bruto[a + 1..b],
        _ => bruto,
    }
    .trim();
    match marca(vendor) {
        Some(m) if !modelo.starts_with(m) => format!("{m} {modelo}"),
        _ => modelo.to_string(),
    }
}

/// O nome do dispositivo na base `pci.ids`.
///
/// Formato: linha de fabricante sem tabulação (`10de  NVIDIA Corporation`),
/// dispositivos com uma tabulação (`\t2204  GA102 [GeForce RTX 3090]`),
/// subsistemas com duas. A busca para no fim do bloco do fabricante.
fn nome_em_pci_ids(base: &str, vendor_id: u32, device_id: u32) -> Option<String> {
    let fabricante = format!("{vendor_id:04x}  ");
    let dispositivo = format!("\t{device_id:04x}  ");
    let mut dentro = false;
    for linha in base.lines() {
        if linha.starts_with('#') || linha.is_empty() {
            continue;
        }
        if !linha.starts_with('\t') {
            if dentro {
                return None;
            }
            dentro = linha.starts_with(&fabricante);
            continue;
        }
        if dentro && let Some(nome) = linha.strip_prefix(&dispositivo) {
            return Some(nome.trim().to_string());
        }
    }
    None
}

/// Lê a base `pci.ids` uma vez, só se alguma placa precisar de nome.
fn leitor_pci_ids() -> impl FnMut(u32, u32) -> Option<String> {
    let mut base: Option<Option<String>> = None;
    move |vendor_id, device_id| {
        let texto = base.get_or_insert_with(|| {
            PCI_IDS
                .iter()
                .find_map(|caminho| std::fs::read_to_string(caminho).ok())
        });
        nome_em_pci_ids(texto.as_deref()?, vendor_id, device_id)
    }
}

fn enumerar(nvml: Option<&Nvml>) -> Vec<Placa> {
    let nvml_placas = nvml.map(crate::nvml::placas).unwrap_or_default();
    juntar(cartoes(Path::new(DRM)), nvml_placas, &mut leitor_pci_ids())
}

pub fn detect() -> Vec<GpuInfo> {
    let nvml = Nvml::init().ok();
    enumerar(nvml.as_ref())
        .into_iter()
        .map(|p| p.info)
        .collect()
}

// ---------------------------------------------------------------------------
// Telemetria
// ---------------------------------------------------------------------------

/// De onde vem a telemetria de uma GPU do perfil.
enum Fonte {
    Nvml(u32),
    /// Os arquivos do amdgpu em `…/device`.
    Amdgpu(PathBuf),
    /// Placa sem leitura (Intel, NVIDIA sem NVML): só a VRAM do perfil.
    Nenhuma,
}

/// Estado persistente de telemetria de GPU no Linux: o NVML aberto uma vez e
/// a fonte de cada GPU do perfil resolvida na criação, não a cada amostra.
pub struct LinuxGpuMonitor {
    nvml: Option<Nvml>,
    /// Uma fonte por GPU do perfil, NA MESMA ORDEM (contrato com a UI).
    fontes: Vec<Fonte>,
}

impl LinuxGpuMonitor {
    pub fn new(profile_gpus: &[GpuInfo]) -> Self {
        let nvml = Nvml::init().ok();
        let placas = enumerar(nvml.as_ref());
        let fontes = casar(profile_gpus, &placas)
            .into_iter()
            .map(|achada| match achada.map(|i| &placas[i]) {
                Some(Placa {
                    nvml_index: Some(i),
                    ..
                }) => Fonte::Nvml(*i),
                Some(Placa {
                    info,
                    device: Some(d),
                    ..
                }) if info.vendor == GpuVendor::Amd => Fonte::Amdgpu(d.clone()),
                _ => Fonte::Nenhuma,
            })
            .collect();
        Self { nvml, fontes }
    }

    /// Telemetria por GPU (na mesma ordem de `profile_gpus`) + a temperatura
    /// da primeira GPU que reportar uma.
    pub fn sample(&mut self, profile_gpus: &[GpuInfo]) -> (Vec<GpuTelemetry>, Option<f32>) {
        let mut temp_c: Option<f32> = None;
        let mut out = Vec::with_capacity(profile_gpus.len());
        for (i, gpu) in profile_gpus.iter().enumerate() {
            let mut t = GpuTelemetry {
                util_percent: None,
                vram_used_bytes: None,
                vram_total_bytes: gpu.vram_total_bytes,
                power_w: None,
                power_limit_w: None,
            };
            match self.fontes.get(i) {
                Some(Fonte::Nvml(idx)) => {
                    if let Some(dev) = self
                        .nvml
                        .as_ref()
                        .and_then(|n| n.device_by_index(*idx).ok())
                    {
                        let a = crate::nvml::amostra(&dev);
                        t.util_percent = a.util_percent;
                        t.vram_used_bytes = a.vram_used_bytes;
                        t.vram_total_bytes = a.vram_total_bytes.unwrap_or(t.vram_total_bytes);
                        t.power_w = a.power_w;
                        t.power_limit_w = a.power_limit_w;
                        temp_c = temp_c.or(a.temp_c);
                    }
                }
                Some(Fonte::Amdgpu(device)) => {
                    let a = amostra_amdgpu(device);
                    t.util_percent = a.util_percent;
                    t.vram_used_bytes = a.vram_used_bytes;
                    t.vram_total_bytes = a.vram_total_bytes.unwrap_or(t.vram_total_bytes);
                    t.power_w = a.power_w;
                    t.power_limit_w = a.power_limit_w;
                    temp_c = temp_c.or(a.temp_c);
                }
                Some(Fonte::Nenhuma) | None => {}
            }
            out.push(t);
        }
        (out, temp_c)
    }
}

/// Para cada GPU do perfil, o índice da placa enumerada que a descreve: pelo
/// mesmo fabricante e nome; senão, a próxima livre do mesmo fabricante. A
/// ordem da enumeração é a mesma do perfil quando nada mudou, mas uma placa
/// que sumiu (driver descarregado) não pode deslocar a telemetria das outras.
fn casar(profile_gpus: &[GpuInfo], placas: &[Placa]) -> Vec<Option<usize>> {
    let mut usada = vec![false; placas.len()];
    profile_gpus
        .iter()
        .map(|gpu| {
            let mesma_marca = |j: usize| !usada[j] && placas[j].info.vendor == gpu.vendor;
            let achada = (0..placas.len())
                .find(|&j| mesma_marca(j) && placas[j].info.name.eq_ignore_ascii_case(&gpu.name))
                .or_else(|| (0..placas.len()).find(|&j| mesma_marca(j)))?;
            usada[achada] = true;
            Some(achada)
        })
        .collect()
}

/// O que o amdgpu expõe em arquivos: ocupação, VRAM e os sensores `hwmon`.
fn amostra_amdgpu(device: &Path) -> crate::nvml::Amostra {
    let hwmon = std::fs::read_dir(device.join("hwmon"))
        .ok()
        .and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path())));
    let sensor = |arquivo: &str| hwmon.as_ref().and_then(|h| ler_u64(&h.join(arquivo)));
    // Microwatts: `power1_input` nas gerações novas, `power1_average` nas
    // anteriores.
    let micro_w = |uw: u64| uw.div_ceil(1_000_000) as u32;
    crate::nvml::Amostra {
        util_percent: ler_u64(&device.join("gpu_busy_percent")).map(|p| p as f32),
        vram_used_bytes: ler_u64(&device.join("mem_info_vram_used")),
        vram_total_bytes: ler_u64(&device.join("mem_info_vram_total")),
        // Milésimos de grau; `temp1` é o sensor "edge", o que as ferramentas
        // de sistema mostram.
        temp_c: sensor("temp1_input").map(|m| m as f32 / 1000.0),
        power_w: sensor("power1_input")
            .or_else(|| sensor("power1_average"))
            .map(micro_w),
        power_limit_w: sensor("power1_cap").map(micro_w),
    }
}

// ---------------------------------------------------------------------------
// Leitura de arquivos do sysfs
// ---------------------------------------------------------------------------

fn ler_texto(caminho: &Path) -> Option<String> {
    let texto = std::fs::read_to_string(caminho).ok()?;
    let texto = texto.trim();
    (!texto.is_empty()).then(|| texto.to_string())
}

fn ler_u64(caminho: &Path) -> Option<u64> {
    ler_texto(caminho)?.parse().ok()
}

/// `0x10de` → 0x10DE.
fn ler_hex(caminho: &Path) -> Option<u32> {
    let texto = ler_texto(caminho)?;
    u32::from_str_radix(texto.trim_start_matches("0x"), 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um `/sys/class/drm` de mentira: `cardN` → `device` apontando para um
    /// dispositivo PCI com os arquivos dados.
    fn cartao(drm: &Path, pci_root: &Path, n: u32, pci: &str, arquivos: &[(&str, &str)]) {
        let dev = pci_root.join(pci);
        std::fs::create_dir_all(&dev).unwrap();
        for (nome, conteudo) in arquivos {
            let caminho = dev.join(nome);
            std::fs::create_dir_all(caminho.parent().unwrap()).unwrap();
            std::fs::write(caminho, conteudo).unwrap();
        }
        let card = drm.join(format!("card{n}"));
        std::fs::create_dir_all(&card).unwrap();
        std::os::unix::fs::symlink(&dev, card.join("device")).unwrap();
    }

    fn nvml_3090(index: u32, pci: &str) -> crate::nvml::Placa {
        crate::nvml::Placa {
            index,
            pci: Some(pci.into()),
            info: GpuInfo {
                name: "NVIDIA GeForce RTX 3090".into(),
                vendor: GpuVendor::Nvidia,
                vram_total_bytes: 24 * GIB,
                is_integrated: false,
                driver_version: Some("610.43.03".into()),
                cuda_compute: Some((8, 6)),
                bandwidth_bytes_s: Some(936_000_000_000),
            },
        }
    }

    const PCI_IDS_TRECHO: &str = "\
# comentário
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]
\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]
\t\t1002 0e3a  Radeon RX 6900 XT
10de  NVIDIA Corporation
\t2204  GA102 [GeForce RTX 3090]
8086  Intel Corporation
\t4680  AlderLake-S GT1 [UHD Graphics 770]
C 03  Display controller
";

    /// A máquina do Pedro: uma 3090 com NVML. O sysfs acha a placa, o NVML
    /// dá os números, e a telemetria sai pelo NVML.
    #[test]
    fn an_nvidia_card_with_nvml_takes_the_nvml_numbers() {
        let tmp = tempfile::tempdir().unwrap();
        let (drm, pci) = (tmp.path().join("drm"), tmp.path().join("pci"));
        cartao(
            &drm,
            &pci,
            0,
            "0000:05:00.0",
            &[
                ("vendor", "0x10de\n"),
                ("device", "0x2204\n"),
                ("class", "0x030000\n"),
            ],
        );
        let placas = juntar(
            cartoes(&drm),
            vec![nvml_3090(0, "0000:05:00.0")],
            &mut |_, _| panic!("com NVML não se consulta o pci.ids"),
        );
        assert_eq!(placas.len(), 1);
        assert_eq!(placas[0].info.name, "NVIDIA GeForce RTX 3090");
        assert_eq!(placas[0].info.vram_total_bytes, 24 * GIB);
        assert_eq!(placas[0].info.cuda_compute, Some((8, 6)));
        assert_eq!(placas[0].nvml_index, Some(0));
    }

    /// Conectores, nós de render e o `simpledrm` (sem PCI) não são placas.
    #[test]
    fn connectors_render_nodes_and_non_pci_devices_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (drm, pci) = (tmp.path().join("drm"), tmp.path().join("pci"));
        cartao(
            &drm,
            &pci,
            1,
            "0000:03:00.0",
            &[
                ("vendor", "0x1002"),
                ("device", "0x73bf"),
                ("class", "0x030000"),
                ("mem_info_vram_total", "17163091968"),
            ],
        );
        std::fs::create_dir_all(drm.join("card1-DP-1")).unwrap();
        std::fs::create_dir_all(drm.join("renderD128")).unwrap();
        // simpledrm: `device` sem `vendor`.
        std::fs::create_dir_all(drm.join("card0/device")).unwrap();

        let achados = cartoes(&drm);
        assert_eq!(achados.len(), 1);
        assert_eq!(achados[0].pci, "0000:03:00.0");
        assert_eq!(achados[0].vendor_id, VENDOR_ID_AMD);
    }

    /// Uma controladora que não é de vídeo (classe diferente de 0x03) fica
    /// de fora mesmo com `vendor` de fabricante de GPU.
    #[test]
    fn non_display_pci_classes_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (drm, pci) = (tmp.path().join("drm"), tmp.path().join("pci"));
        cartao(
            &drm,
            &pci,
            0,
            "0000:07:00.1",
            &[
                ("vendor", "0x10de"),
                ("device", "0x1aef"),
                ("class", "0x040300"),
            ],
        );
        assert!(cartoes(&drm).is_empty());
    }

    /// AMD dedicada: VRAM do amdgpu, nome comercial do pci.ids.
    #[test]
    fn an_amd_card_gets_vram_from_sysfs_and_name_from_pci_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let (drm, pci) = (tmp.path().join("drm"), tmp.path().join("pci"));
        cartao(
            &drm,
            &pci,
            0,
            "0000:03:00.0",
            &[
                ("vendor", "0x1002"),
                ("device", "0x73bf"),
                ("class", "0x030000"),
                ("mem_info_vram_total", "17163091968"),
            ],
        );
        let placas = juntar(cartoes(&drm), vec![], &mut |v, d| {
            nome_em_pci_ids(PCI_IDS_TRECHO, v, d)
        });
        let info = &placas[0].info;
        assert_eq!(info.name, "AMD Radeon RX 6800/6800 XT / 6900 XT");
        assert_eq!(info.vendor, GpuVendor::Amd);
        assert_eq!(info.vram_total_bytes, 17_163_091_968);
        assert!(!info.is_integrated);
    }

    /// A integrada da AMD reserva pouca memória; a da Intel nem informa.
    #[test]
    fn apus_and_intel_graphics_count_as_integrated() {
        let tmp = tempfile::tempdir().unwrap();
        let (drm, pci) = (tmp.path().join("drm"), tmp.path().join("pci"));
        cartao(
            &drm,
            &pci,
            0,
            "0000:05:00.0",
            &[
                ("vendor", "0x1002"),
                ("device", "0x1638"),
                ("class", "0x030000"),
                ("mem_info_vram_total", "536870912"),
            ],
        );
        cartao(
            &drm,
            &pci,
            1,
            "0000:00:02.0",
            &[
                ("vendor", "0x8086"),
                ("device", "0x4680"),
                ("class", "0x030000"),
            ],
        );
        let placas = juntar(cartoes(&drm), vec![], &mut |v, d| {
            nome_em_pci_ids(PCI_IDS_TRECHO, v, d)
        });
        assert_eq!(placas.len(), 2);
        assert!(placas[0].info.is_integrated);
        assert_eq!(
            placas[0].info.name,
            "AMD Radeon Vega Series / Radeon Vega Mobile Series"
        );
        assert!(placas[1].info.is_integrated);
        assert_eq!(placas[1].info.name, "Intel UHD Graphics 770");
        assert_eq!(placas[1].info.vram_total_bytes, 0);
    }

    /// Sem NVML (nouveau) a NVIDIA ainda aparece, com o nome do pci.ids.
    #[test]
    fn an_nvidia_card_without_nvml_still_shows_up() {
        let tmp = tempfile::tempdir().unwrap();
        let (drm, pci) = (tmp.path().join("drm"), tmp.path().join("pci"));
        cartao(
            &drm,
            &pci,
            0,
            "0000:05:00.0",
            &[
                ("vendor", "0x10de"),
                ("device", "0x2204"),
                ("class", "0x030000"),
            ],
        );
        let placas = juntar(cartoes(&drm), vec![], &mut |v, d| {
            nome_em_pci_ids(PCI_IDS_TRECHO, v, d)
        });
        assert_eq!(placas[0].info.name, "NVIDIA GeForce RTX 3090");
        assert_eq!(placas[0].info.vendor, GpuVendor::Nvidia);
        assert!(!placas[0].info.is_integrated);
        assert_eq!(placas[0].nvml_index, None);
    }

    /// Num contêiner sem `/sys/class/drm` o NVML ainda enxerga a placa — e
    /// ela não pode sumir do perfil.
    #[test]
    fn nvml_cards_missing_from_sysfs_are_kept() {
        let placas = juntar(vec![], vec![nvml_3090(0, "0000:05:00.0")], &mut |_, _| None);
        assert_eq!(placas.len(), 1);
        assert_eq!(placas[0].nvml_index, Some(0));
        assert!(placas[0].device.is_none());
    }

    #[test]
    fn the_pci_ids_lookup_stays_inside_the_vendor_block() {
        assert_eq!(
            nome_em_pci_ids(PCI_IDS_TRECHO, 0x10de, 0x2204).as_deref(),
            Some("GA102 [GeForce RTX 3090]")
        );
        // 0x73bf existe, mas é da AMD — não da NVIDIA.
        assert_eq!(nome_em_pci_ids(PCI_IDS_TRECHO, 0x10de, 0x73bf), None);
        assert_eq!(nome_em_pci_ids(PCI_IDS_TRECHO, 0x1234, 0x0001), None);
    }

    #[test]
    fn friendly_names_keep_the_commercial_part() {
        assert_eq!(
            nome_amigavel(GpuVendor::Nvidia, "GA102 [GeForce RTX 3090]"),
            "NVIDIA GeForce RTX 3090"
        );
        assert_eq!(
            nome_amigavel(GpuVendor::Intel, "Alder Lake-P GT2 [Iris Xe Graphics]"),
            "Intel Iris Xe Graphics"
        );
        // Sem colchetes, o nome inteiro; sem marca conhecida, sem prefixo.
        assert_eq!(nome_amigavel(GpuVendor::Other, "Virtio GPU"), "Virtio GPU");
    }

    /// A telemetria de uma AMD sai dos arquivos do amdgpu, em unidades da tela.
    #[test]
    fn amdgpu_telemetry_reads_busy_vram_and_hwmon() {
        let tmp = tempfile::tempdir().unwrap();
        let dev = tmp.path();
        for (nome, conteudo) in [
            ("gpu_busy_percent", "37\n"),
            ("mem_info_vram_used", "4294967296\n"),
            ("mem_info_vram_total", "17163091968\n"),
            ("hwmon/hwmon3/temp1_input", "52000\n"),
            ("hwmon/hwmon3/power1_average", "118000000\n"),
            ("hwmon/hwmon3/power1_cap", "255000000\n"),
        ] {
            let caminho = dev.join(nome);
            std::fs::create_dir_all(caminho.parent().unwrap()).unwrap();
            std::fs::write(caminho, conteudo).unwrap();
        }
        let a = amostra_amdgpu(dev);
        assert_eq!(a.util_percent, Some(37.0));
        assert_eq!(a.vram_used_bytes, Some(4 * GIB));
        assert_eq!(a.vram_total_bytes, Some(17_163_091_968));
        assert_eq!(a.temp_c, Some(52.0));
        assert_eq!(a.power_w, Some(118));
        assert_eq!(a.power_limit_w, Some(255));
    }

    /// Sem os arquivos (Intel, driver sem hwmon), tudo `None` — nunca zero
    /// inventado.
    #[test]
    fn missing_sysfs_files_become_none() {
        let tmp = tempfile::tempdir().unwrap();
        let a = amostra_amdgpu(tmp.path());
        assert_eq!(a.util_percent, None);
        assert_eq!(a.vram_used_bytes, None);
        assert_eq!(a.temp_c, None);
        assert_eq!(a.power_w, None);
    }

    /// Duas placas do mesmo nome: cada GPU do perfil fica com uma.
    #[test]
    fn matching_gives_each_profile_gpu_its_own_card() {
        let placa = |nome: &str, vendor| Placa {
            info: GpuInfo {
                name: nome.into(),
                vendor,
                vram_total_bytes: 0,
                is_integrated: false,
                driver_version: None,
                cuda_compute: None,
                bandwidth_bytes_s: None,
            },
            nvml_index: None,
            device: None,
        };
        let placas = vec![
            placa("Intel UHD Graphics 770", GpuVendor::Intel),
            placa("NVIDIA GeForce RTX 3090", GpuVendor::Nvidia),
            placa("NVIDIA GeForce RTX 3090", GpuVendor::Nvidia),
        ];
        let perfil: Vec<GpuInfo> = [2usize, 1, 0]
            .iter()
            .map(|&i| placas[i].info.clone())
            .collect();
        assert_eq!(casar(&perfil, &placas), vec![Some(1), Some(2), Some(0)]);

        // Uma placa que sumiu não desloca as outras.
        let so_intel = vec![placas[0].clone()];
        assert_eq!(casar(&perfil, &so_intel), vec![None, None, Some(0)]);
    }

    /// Na máquina real: a enumeração não entra em pânico, e toda placa
    /// que aparece tem nome.
    #[test]
    fn this_machine_enumerates_without_panicking() {
        for gpu in detect() {
            assert!(!gpu.name.is_empty());
        }
    }
}
