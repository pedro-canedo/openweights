//! Leitor mínimo das sequências que interessam ao gerente de sessões.
//!
//! Não é um emulador: o terminal de verdade roda na interface (xterm.js).
//! Aqui só se pesca, na saída crua, o que a lista de sessões precisa saber
//! mesmo com a tela fechada:
//!
//! - OSC 0 / OSC 2 — o título que o programa deu a si mesmo;
//! - OSC 7 — a pasta atual (`file://host/caminho`), que shells modernos anunciam;
//! - OSC 9 — "preciso de você" (o Codex/OwCLI manda ao pedir aprovação e ao
//!   terminar a vez, com `tui.notification_method = "osc9"`);
//! - OSC 777;notify — a variante do rxvt/foot para o mesmo recado;
//! - BEL solto — o sino, a alternativa quando o OSC 9 não passa (ConPTY antigo).
//!
//! A sequência pode chegar partida entre duas leituras do pseudoterminal: o
//! estado atravessa os blocos. O BEL que termina uma OSC não é sino.

/// O que foi reconhecido num bloco de saída.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sinal {
    Titulo(String),
    Pasta(String),
    Notificacao(String),
    Sino,
}

/// Teto de uma OSC: título e notificação são curtos; passar disso é lixo
/// (ou um OSC 52 de área de transferência, que não nos interessa).
const TETO: usize = 4096;

#[derive(Default)]
pub(crate) struct Leitor {
    estado: Estado,
    corpo: Vec<u8>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Estado {
    #[default]
    Texto,
    /// Viu ESC; o próximo decide.
    Esc,
    /// Dentro de `ESC ]` … até BEL ou `ESC \`.
    Osc,
    /// Dentro da OSC, viu ESC (pode ser o `ST`).
    OscEsc,
}

impl Leitor {
    pub(crate) fn ler(&mut self, bloco: &[u8]) -> Vec<Sinal> {
        let mut saida = Vec::new();
        for &b in bloco {
            match self.estado {
                Estado::Texto => match b {
                    0x1b => self.estado = Estado::Esc,
                    0x07 => saida.push(Sinal::Sino),
                    _ => {}
                },
                Estado::Esc => {
                    if b == b']' {
                        self.estado = Estado::Osc;
                        self.corpo.clear();
                    } else if b == 0x1b {
                        // ESC ESC: continua esperando.
                    } else {
                        self.estado = Estado::Texto;
                    }
                }
                Estado::Osc => match b {
                    0x07 => self.fechar(&mut saida),
                    0x1b => self.estado = Estado::OscEsc,
                    _ => {
                        if self.corpo.len() < TETO {
                            self.corpo.push(b);
                        }
                    }
                },
                Estado::OscEsc => {
                    if b == b'\\' {
                        self.fechar(&mut saida);
                    } else {
                        // ESC no meio da OSC sem ser ST: a OSC acabou mal;
                        // tratar o ESC como começo de outra sequência.
                        self.corpo.clear();
                        self.estado = if b == b']' {
                            Estado::Osc
                        } else {
                            Estado::Texto
                        };
                    }
                }
            }
        }
        saida
    }

    fn fechar(&mut self, saida: &mut Vec<Sinal>) {
        self.estado = Estado::Texto;
        if self.corpo.len() >= TETO {
            self.corpo.clear();
            return;
        }
        let texto = String::from_utf8_lossy(&self.corpo).into_owned();
        self.corpo.clear();
        let (codigo, resto) = texto.split_once(';').unwrap_or((texto.as_str(), ""));
        match codigo {
            "0" | "2" => saida.push(Sinal::Titulo(resto.to_string())),
            "7" => {
                if let Some(pasta) = pasta_de_url(resto) {
                    saida.push(Sinal::Pasta(pasta));
                }
            }
            // OSC 9;4 é a barra de progresso do ConEmu/Windows Terminal, não
            // um pedido de atenção.
            "9" if !resto.starts_with("4;") => saida.push(Sinal::Notificacao(resto.to_string())),
            "777" => {
                if let Some(msg) = resto.strip_prefix("notify;") {
                    let corpo = msg.split_once(';').map_or(msg, |(titulo, corpo)| {
                        if corpo.is_empty() { titulo } else { corpo }
                    });
                    saida.push(Sinal::Notificacao(corpo.to_string()));
                }
            }
            _ => {}
        }
    }
}

/// `file://host/caminho%20com%20espaco` → `/caminho com espaco`.
fn pasta_de_url(url: &str) -> Option<String> {
    let sem_esquema = url.strip_prefix("file://")?;
    let caminho = &sem_esquema[sem_esquema.find('/')?..];
    let bytes = caminho.as_bytes();
    let mut saida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            saida.push(v);
            i += 3;
            continue;
        }
        saida.push(bytes[i]);
        i += 1;
    }
    let pasta = String::from_utf8(saida).ok()?;
    // No Windows o shell manda `file://host/C:/pasta`.
    let pasta = match pasta.as_bytes() {
        [b'/', letra, b':', ..] if letra.is_ascii_alphabetic() => pasta[1..].to_string(),
        _ => pasta,
    };
    Some(pasta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ler(pedacos: &[&[u8]]) -> Vec<Sinal> {
        let mut l = Leitor::default();
        pedacos.iter().flat_map(|p| l.ler(p)).collect()
    }

    #[test]
    fn titulo_por_bel_e_por_st() {
        assert_eq!(
            ler(&[b"\x1b]0;vim soma.py\x07"]),
            vec![Sinal::Titulo("vim soma.py".into())]
        );
        assert_eq!(
            ler(&[b"\x1b]2;OwCLI\x1b\\"]),
            vec![Sinal::Titulo("OwCLI".into())]
        );
    }

    #[test]
    fn sequencia_partida_entre_leituras() {
        assert_eq!(
            ler(&[b"texto \x1b]", b"9;Aprovar o comando?", b"\x07 mais"]),
            vec![Sinal::Notificacao("Aprovar o comando?".into())]
        );
        assert_eq!(
            ler(&[b"\x1b", b"]2;a\x1b", b"\\"]),
            vec![Sinal::Titulo("a".into())]
        );
    }

    #[test]
    fn bel_que_fecha_osc_nao_e_sino_mas_o_solto_e() {
        assert_eq!(
            ler(&[b"\x1b]0;x\x07ok\x07"]),
            vec![Sinal::Titulo("x".into()), Sinal::Sino]
        );
    }

    #[test]
    fn pasta_do_osc7_decodificada() {
        assert_eq!(
            ler(&[b"\x1b]7;file://maquina/home/p/meu%20projeto\x07"]),
            vec![Sinal::Pasta("/home/p/meu projeto".into())]
        );
        assert_eq!(
            ler(&[b"\x1b]7;file://pc/C:/Users/p\x1b\\"]),
            vec![Sinal::Pasta("C:/Users/p".into())]
        );
    }

    #[test]
    fn progresso_do_windows_terminal_nao_e_notificacao() {
        assert_eq!(ler(&[b"\x1b]9;4;1;50\x07"]), vec![]);
    }

    #[test]
    fn notify_do_osc_777() {
        assert_eq!(
            ler(&[b"\x1b]777;notify;OwCLI;Tarefa concluida\x07"]),
            vec![Sinal::Notificacao("Tarefa concluida".into())]
        );
    }

    #[test]
    fn osc_gigante_e_descartada_sem_crescer_sem_limite() {
        let mut enorme = b"\x1b]52;c;".to_vec();
        enorme.extend(std::iter::repeat_n(b'A', 100_000));
        enorme.push(0x07);
        assert_eq!(
            ler(&[&enorme, b"\x1b]0;depois\x07"]),
            vec![Sinal::Titulo("depois".into())]
        );
    }

    #[test]
    fn csi_comum_nao_confunde_o_leitor() {
        assert_eq!(ler(&[b"\x1b[31mvermelho\x1b[0m\x1b[?1049h"]), vec![]);
    }
}
