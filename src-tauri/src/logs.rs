//! O log de tudo que roda por baixo do app, num lugar só.
//!
//! O llama-server, o decisor, o 9router e o próprio OpenWeights escrevem cada
//! um no seu canto; antes só o evento `server-log` chegava à tela, e quem abria
//! a tela depois do boot perdia o que importava (o erro da subida). Aqui cada
//! linha entra num anel por origem, com número de sequência, e sai para a tela
//! por `log-line`; `server_logs(since)` devolve o que já passou.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Linhas guardadas por origem: uma origem tagarela não expulsa as outras.
const POR_ORIGEM: usize = 2000;
/// Uma linha maior que isto é cortada (um JSON gigante não trava a tela).
const MAX_LINHA: usize = 4000;

/// As origens que a tela conhece, na ordem em que aparecem.
pub const SERVIDOR: &str = "servidor";
pub const DECISOR: &str = "decisor";
pub const NINEROUTER: &str = "9router";
pub const GATEWAY: &str = "gateway";
pub const APP: &str = "app";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Linha {
    pub seq: u64,
    /// Milissegundos desde a época Unix.
    pub ts: u64,
    pub origem: String,
    pub texto: String,
}

#[derive(Default)]
pub struct Anel {
    proximo: u64,
    por_origem: BTreeMap<String, VecDeque<Linha>>,
}

impl Anel {
    pub fn empurrar(&mut self, origem: &str, texto: &str, ts: u64) -> Linha {
        self.proximo += 1;
        let mut texto = texto.trim_end_matches(['\r', '\n']).to_string();
        if texto.len() > MAX_LINHA {
            let mut corte = MAX_LINHA;
            while !texto.is_char_boundary(corte) {
                corte -= 1;
            }
            texto.truncate(corte);
            texto.push('…');
        }
        let linha = Linha {
            seq: self.proximo,
            ts,
            origem: origem.to_string(),
            texto,
        };
        let fila = self.por_origem.entry(origem.to_string()).or_default();
        if fila.len() >= POR_ORIGEM {
            fila.pop_front();
        }
        fila.push_back(linha.clone());
        linha
    }

    /// As linhas com `seq` maior que `desde`, em ordem, de uma origem ou de todas.
    pub fn ler(&self, desde: u64, origem: Option<&str>) -> Vec<Linha> {
        let mut saida: Vec<Linha> = self
            .por_origem
            .iter()
            .filter(|(o, _)| origem.is_none_or(|f| f == o.as_str()))
            .flat_map(|(_, fila)| fila.iter())
            .filter(|l| l.seq > desde)
            .cloned()
            .collect();
        saida.sort_by_key(|l| l.seq);
        saida
    }

    pub fn limpar(&mut self, origem: Option<&str>) {
        match origem {
            Some(o) => {
                self.por_origem.remove(o);
            }
            None => self.por_origem.clear(),
        }
    }
}

static ANEL: OnceLock<Mutex<Anel>> = OnceLock::new();
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

fn anel() -> &'static Mutex<Anel> {
    ANEL.get_or_init(|| Mutex::new(Anel::default()))
}

/// Liga a emissão de eventos; antes disso as linhas só entram no anel.
pub fn ligar(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());
}

fn agora_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn registrar(origem: &str, texto: &str) {
    let linha =
        anel()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .empurrar(origem, texto, agora_ms());
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit("log-line", &linha);
    }
}

/// O que já passou, para a tela que abriu depois do boot.
#[tauri::command]
pub fn server_logs(since: Option<u64>, source: Option<String>) -> Vec<Linha> {
    anel()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ler(since.unwrap_or(0), source.as_deref())
}

#[tauri::command]
pub fn logs_clear(source: Option<String>) {
    anel()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .limpar(source.as_deref());
}

/// Encaminha o `log::` do próprio app (nível `warn` para cima e o que o filtro
/// do `env_logger` deixar passar) para a origem `app`, sem tirar nada do stderr.
pub struct Tee(pub env_logger::Logger);

impl log::Log for Tee {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.0.enabled(m)
    }
    fn log(&self, r: &log::Record) {
        if self.0.matches(r) {
            registrar(APP, &format!("{} {}", r.level(), r.args()));
        }
        self.0.log(r);
    }
    fn flush(&self) {
        self.0.flush();
    }
}

pub fn iniciar_logger() {
    let inner =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    let nivel = inner.filter();
    if log::set_boxed_logger(Box::new(Tee(inner))).is_ok() {
        log::set_max_level(nivel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sequencia_e_global_e_a_leitura_respeita_desde_e_origem() {
        let mut a = Anel::default();
        a.empurrar("servidor", "um", 1);
        a.empurrar("decisor", "dois", 2);
        a.empurrar("servidor", "três", 3);
        assert_eq!(
            a.ler(0, None).iter().map(|l| l.seq).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(a.ler(1, None).len(), 2);
        let so_servidor = a.ler(0, Some("servidor"));
        assert_eq!(so_servidor.len(), 2);
        assert!(so_servidor.iter().all(|l| l.origem == "servidor"));
    }

    #[test]
    fn uma_origem_tagarela_nao_expulsa_as_outras() {
        let mut a = Anel::default();
        a.empurrar("9router", "importante", 0);
        for i in 0..(POR_ORIGEM + 50) {
            a.empurrar("servidor", &format!("linha {i}"), 0);
        }
        assert_eq!(a.ler(0, Some("servidor")).len(), POR_ORIGEM);
        assert_eq!(a.ler(0, Some("9router")).len(), 1);
        // O que ficou do servidor são as mais novas.
        assert!(
            a.ler(0, Some("servidor"))
                .last()
                .unwrap()
                .texto
                .ends_with("2049")
        );
    }

    #[test]
    fn linha_gigante_e_cortada_em_fronteira_de_caractere() {
        let mut a = Anel::default();
        let l = a.empurrar("app", &"é".repeat(MAX_LINHA), 0);
        assert!(l.texto.ends_with('…'));
        assert!(l.texto.len() <= MAX_LINHA + '…'.len_utf8());
    }

    #[test]
    fn fim_de_linha_nao_entra_e_limpar_esvazia() {
        let mut a = Anel::default();
        assert_eq!(a.empurrar("app", "oi\r\n", 0).texto, "oi");
        a.limpar(Some("app"));
        assert!(a.ler(0, None).is_empty());
        a.empurrar("app", "x", 0);
        a.limpar(None);
        assert!(a.ler(0, None).is_empty());
    }
}
