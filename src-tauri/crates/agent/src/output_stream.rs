//! Saída de ferramenta ao vivo, agrupada antes de virar evento.
//!
//! O `spawner` entrega o que leu do cano em pedaços de até 8 KB, um por
//! leitura — um `cargo build` verboso gera centenas por segundo. Cada evento
//! atravessa o IPC do Tauri e faz a interface redesenhar a conversa inteira,
//! então mandar um por leitura deixaria a UI pesada exatamente durante o
//! comando longo que o terminal da sessão existe para mostrar.
//!
//! Aqui o texto se acumula e sai a cada 4 KB ou 100 ms, o que vier primeiro.
//! O agrupamento é reativo — acontece no próprio `push`, sem tarefa de
//! relógio — para não haver nada a cancelar quando o run é interrompido. A
//! contrapartida é que a cauda precisa de um [`ChunkCoalescer::flush`]
//! explícito no fim da chamada; é o que faz a saída de um comando rápido não
//! sumir.

use crate::events::EventSink;
use lr_types::agent::RunEventKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Tamanho a partir do qual vale mandar, mesmo antes do tempo.
const FLUSH_BYTES: usize = 4 * 1024;
/// Intervalo máximo entre dois envios — o que dá sensação de "ao vivo".
const FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// Junta pedaços de saída de uma chamada e os emite como `tool.output`.
pub struct ChunkCoalescer {
    sink: Arc<EventSink>,
    call_id: String,
    pendente: Mutex<(String, Instant)>,
    streamed: AtomicBool,
}

impl ChunkCoalescer {
    pub fn new(sink: Arc<EventSink>, call_id: impl Into<String>) -> Self {
        Self {
            sink,
            call_id: call_id.into(),
            pendente: Mutex::new((String::new(), Instant::now())),
            streamed: AtomicBool::new(false),
        }
    }

    /// Acumula um pedaço; emite quando passa do tamanho ou do intervalo.
    pub fn push(&self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        let pronto = {
            let mut guarda = self.pendente.lock().unwrap();
            guarda.0.push_str(chunk);
            let vencido = guarda.1.elapsed() >= FLUSH_INTERVAL;
            if guarda.0.len() >= FLUSH_BYTES || vencido {
                guarda.1 = Instant::now();
                Some(std::mem::take(&mut guarda.0))
            } else {
                None
            }
        };
        if let Some(texto) = pronto {
            self.emitir(texto);
        }
    }

    /// Manda o que sobrou. Chamar sempre ao fim da chamada — inclusive
    /// quando ela foi cancelada, senão o último trecho se perde.
    pub fn flush(&self) {
        let resto = {
            let mut guarda = self.pendente.lock().unwrap();
            guarda.1 = Instant::now();
            std::mem::take(&mut guarda.0)
        };
        if !resto.is_empty() {
            self.emitir(resto);
        }
    }

    /// Já saiu alguma coisa por aqui?
    ///
    /// É o que impede a saída dobrada: quem fez streaming não repete o corpo
    /// inteiro no fim, porque a interface **concatena** os pedaços.
    pub fn streamed(&self) -> bool {
        self.streamed.load(Ordering::SeqCst)
    }

    /// O sink no formato que o `ToolContext` entende.
    pub fn as_output_sink(self: &Arc<Self>) -> lr_tools::OutputSink {
        let eu = self.clone();
        Arc::new(move |chunk: &str| eu.push(chunk))
    }

    fn emitir(&self, chunk: String) {
        self.streamed.store(true, Ordering::SeqCst);
        self.sink.emit(RunEventKind::ToolOutput {
            call_id: self.call_id.clone(),
            chunk,
            truncated: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::RunEvent;

    /// Coleta os eventos emitidos, para o teste ver o que a UI veria.
    fn espiao() -> (Arc<EventSink>, Arc<Mutex<Vec<RunEvent>>>) {
        let vistos = Arc::new(Mutex::new(Vec::new()));
        let alvo = vistos.clone();
        let sink = EventSink::new("run-1", None)
            .with_listener(Arc::new(move |e: RunEvent| alvo.lock().unwrap().push(e)));
        (Arc::new(sink), vistos)
    }

    fn saidas(vistos: &Arc<Mutex<Vec<RunEvent>>>) -> Vec<String> {
        vistos
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match &e.event {
                RunEventKind::ToolOutput { chunk, .. } => Some(chunk.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn small_chunks_wait_instead_of_flooding_the_interface() {
        let (sink, vistos) = espiao();
        let c = ChunkCoalescer::new(sink, "call-1");
        // O primeiro `push` acontece antes de 100 ms do `new`, então só o
        // tamanho poderia disparar — e não dispara.
        for _ in 0..10 {
            c.push("oi");
        }
        assert!(saidas(&vistos).is_empty(), "não devia ter emitido ainda");
        assert!(!c.streamed());
    }

    #[test]
    fn a_big_chunk_goes_out_immediately() {
        let (sink, vistos) = espiao();
        let c = ChunkCoalescer::new(sink, "call-1");
        c.push(&"x".repeat(FLUSH_BYTES + 1));
        assert_eq!(saidas(&vistos).len(), 1);
        assert!(c.streamed());
    }

    #[test]
    fn the_tail_survives_in_the_final_flush() {
        let (sink, vistos) = espiao();
        let c = ChunkCoalescer::new(sink, "call-1");
        c.push("fim do comando");
        c.flush();
        assert_eq!(saidas(&vistos), vec!["fim do comando".to_string()]);
        assert!(c.streamed());
    }

    #[test]
    fn flushing_nothing_emits_nothing() {
        let (sink, vistos) = espiao();
        let c = ChunkCoalescer::new(sink, "call-1");
        c.flush();
        assert!(saidas(&vistos).is_empty());
        assert!(!c.streamed(), "sem saída não houve streaming");
    }

    #[test]
    fn time_alone_is_enough_to_flush() {
        let (sink, vistos) = espiao();
        let c = ChunkCoalescer::new(sink, "call-1");
        std::thread::sleep(FLUSH_INTERVAL + Duration::from_millis(20));
        c.push("depois da espera");
        assert_eq!(saidas(&vistos), vec!["depois da espera".to_string()]);
    }
}
