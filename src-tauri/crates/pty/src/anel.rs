//! A saída recente de uma sessão, endereçada por posição absoluta.
//!
//! Cada byte que o terminal produz tem um endereço (`offset`) que só cresce.
//! A interface guarda até onde já desenhou; ao voltar (webview recarregada,
//! tela reaberta) pede "desde o byte N" e recebe exatamente o que falta — sem
//! buraco e sem repetir. Se N já saiu do anel, recebe o que sobrou e sabe que
//! o começo se perdeu (`truncado`), para limpar a tela antes de redesenhar.

use std::collections::VecDeque;

pub(crate) struct Anel {
    capacidade: usize,
    bytes: VecDeque<u8>,
    /// Endereço do primeiro byte ainda guardado.
    inicio: u64,
}

/// O que o anel devolve a quem pede "desde N".
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Trecho {
    pub offset: u64,
    pub dados: Vec<u8>,
    /// Parte do que foi pedido já tinha saído do anel.
    pub truncado: bool,
}

impl Anel {
    pub(crate) fn new(capacidade: usize) -> Self {
        Self {
            capacidade,
            bytes: VecDeque::with_capacity(capacidade.min(64 * 1024)),
            inicio: 0,
        }
    }

    /// Endereço do próximo byte a ser produzido.
    pub(crate) fn fim(&self) -> u64 {
        self.inicio + self.bytes.len() as u64
    }

    /// Guarda `dados` e devolve o endereço do primeiro deles.
    pub(crate) fn guardar(&mut self, dados: &[u8]) -> u64 {
        let offset = self.fim();
        if dados.len() >= self.capacidade {
            // Bloco maior que o anel inteiro: só a cauda sobrevive.
            let corte = dados.len() - self.capacidade;
            self.inicio = offset + corte as u64;
            self.bytes.clear();
            self.bytes.extend(&dados[corte..]);
            return offset;
        }
        let excesso = (self.bytes.len() + dados.len()).saturating_sub(self.capacidade);
        if excesso > 0 {
            self.bytes.drain(..excesso);
            self.inicio += excesso as u64;
        }
        self.bytes.extend(dados);
        offset
    }

    /// O que existe a partir de `desde` (ou desde o começo guardado).
    pub(crate) fn desde(&self, desde: Option<u64>) -> Trecho {
        let pedido = desde.unwrap_or(0);
        let de = pedido.clamp(self.inicio, self.fim());
        let pulo = (de - self.inicio) as usize;
        Trecho {
            offset: de,
            dados: self.bytes.iter().skip(pulo).copied().collect(),
            // "Desde o começo" com nada perdido não é truncado; pedir um
            // endereço que já saiu do anel é.
            truncado: pedido < self.inicio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enderecos_crescem_e_o_replay_nao_repete_nem_pula() {
        let mut a = Anel::new(16);
        assert_eq!(a.guardar(b"abc"), 0);
        assert_eq!(a.guardar(b"defg"), 3);
        assert_eq!(a.fim(), 7);
        let t = a.desde(Some(3));
        assert_eq!(
            (t.offset, t.dados.as_slice(), t.truncado),
            (3, &b"defg"[..], false)
        );
        let t = a.desde(Some(7));
        assert!(t.dados.is_empty());
    }

    #[test]
    fn o_que_sai_do_anel_vira_truncado() {
        let mut a = Anel::new(8);
        a.guardar(b"0123456789");
        assert_eq!(a.fim(), 10);
        let t = a.desde(Some(0));
        assert_eq!(
            (t.offset, t.dados.as_slice(), t.truncado),
            (2, &b"23456789"[..], true)
        );
        a.guardar(b"ab");
        let t = a.desde(Some(3));
        assert_eq!(
            (t.offset, t.dados.as_slice(), t.truncado),
            (4, &b"456789ab"[..], true)
        );
    }

    #[test]
    fn desde_o_comeco_sem_perda_nao_e_truncado() {
        let mut a = Anel::new(8);
        a.guardar(b"oi");
        let t = a.desde(None);
        assert_eq!(
            (t.offset, t.dados.as_slice(), t.truncado),
            (0, &b"oi"[..], false)
        );
    }

    #[test]
    fn pedido_no_futuro_devolve_vazio_no_fim() {
        let mut a = Anel::new(8);
        a.guardar(b"oi");
        let t = a.desde(Some(99));
        assert_eq!((t.offset, t.dados.len()), (2, 0));
    }
}
