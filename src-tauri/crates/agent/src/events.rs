//! Trilha de eventos de um run: numeração, persistência e entrega à interface.
//!
//! Cada evento ganha um `seq` monotônico dentro do run. É esse número que
//! permite fechar e reabrir a interface sem perder nada: o replay pede tudo
//! com `seq > último_visto` e continua ao vivo do ponto exato.
//!
//! Deltas de token (texto/raciocínio/saída de ferramenta) NÃO vão ao banco —
//! são milhares por resposta e o conteúdo final chega inteiro em
//! `assistant.message` / `tool.result`.

use lr_store::Store;
use lr_types::agent::{RunEvent, RunEventKind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Entrega de um evento ao vivo. O app passa o `Channel` do Tauri aqui.
pub type EventCallback = Arc<dyn Fn(RunEvent) + Send + Sync>;

/// Relógio em milissegundos (o envelope usa `tsMs`).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Numera, persiste e distribui os eventos de um run.
pub struct EventSink {
    run_id: String,
    seq: AtomicU64,
    store: Option<Arc<Store>>,
    /// Ouvintes ao vivo. O mesmo cadeado protege a gravação, para que um
    /// `attach` no meio do run não veja um evento duas vezes nem pule nenhum.
    listeners: Mutex<Vec<EventCallback>>,
}

impl EventSink {
    pub fn new(run_id: impl Into<String>, store: Option<Arc<Store>>) -> Self {
        Self {
            run_id: run_id.into(),
            seq: AtomicU64::new(0),
            store,
            listeners: Mutex::new(Vec::new()),
        }
    }

    /// Começa a numerar a partir de `seq` (retomada de um run já gravado).
    pub fn resuming_from(self, seq: u64) -> Self {
        self.seq.store(seq, Ordering::SeqCst);
        self
    }

    pub fn with_listener(self, cb: EventCallback) -> Self {
        self.listeners.lock().unwrap().push(cb);
        self
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Último `seq` emitido (0 = nada ainda).
    pub fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }

    /// Emite um evento: numera, grava (quando for o caso) e entrega a todos.
    pub fn emit(&self, event: RunEventKind) -> RunEvent {
        let ev = RunEvent {
            run_id: self.run_id.clone(),
            seq: self.seq.fetch_add(1, Ordering::SeqCst) + 1,
            ts_ms: now_ms(),
            event,
        };
        // Cadeado tomado antes de gravar: `attach` faz o replay dentro dele e
        // por isso nunca cruza com uma emissão pela metade.
        let listeners = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
        if ev.is_persistable()
            && let Some(store) = &self.store
            && let Err(e) = store.append_run_event(&ev)
        {
            log::warn!("não foi possível gravar o evento {}: {e}", ev.kind_name());
        }
        for cb in listeners.iter() {
            cb(ev.clone());
        }
        ev
    }

    /// Liga um novo ouvinte, entregando antes tudo que ele perdeu.
    ///
    /// O replay sai do banco (os deltas não estão lá — a interface recompõe o
    /// texto pelas mensagens completas).
    pub fn attach(&self, after_seq: u64, cb: EventCallback) {
        let mut listeners = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = &self.store {
            match store.list_run_events(&self.run_id, after_seq) {
                Ok(rows) => {
                    for row in rows {
                        match serde_json::from_str::<RunEvent>(&row.payload_json) {
                            Ok(ev) => cb(ev),
                            Err(e) => log::warn!("evento {} ilegível no replay: {e}", row.seq),
                        }
                    }
                }
                Err(e) => log::warn!("falha ao reler os eventos do run: {e}"),
            }
        }
        listeners.push(cb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{RunMode, RunStatus, UsageStats};

    fn collector() -> (EventCallback, Arc<Mutex<Vec<RunEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let cb: EventCallback = Arc::new(move |ev| sink_seen.lock().unwrap().push(ev));
        (cb, seen)
    }

    fn started() -> RunEventKind {
        RunEventKind::RunStarted {
            chat_id: 1,
            model: "m".into(),
            mode: RunMode::Approve,
            yolo: false,
            workspace_dir: None,
            tools: vec![],
        }
    }

    #[test]
    fn seq_is_monotonic_and_events_reach_the_listener() {
        let (cb, seen) = collector();
        let sink = EventSink::new("r1", None).with_listener(cb);

        sink.emit(started());
        sink.emit(RunEventKind::StepStarted {
            step_id: "s1".into(),
            index: 1,
        });
        sink.emit(RunEventKind::AssistantDelta {
            step_id: "s1".into(),
            text: "oi".into(),
        });

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(
            seen.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(seen.iter().all(|e| e.run_id == "r1"));
        assert_eq!(sink.last_seq(), 3);
    }

    #[test]
    fn deltas_stay_out_of_the_database_but_messages_are_saved() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .create_run("r1", None, "m", RunMode::Approve, false, None, "p")
            .unwrap();
        let sink = EventSink::new("r1", Some(store.clone()));

        sink.emit(started());
        sink.emit(RunEventKind::AssistantDelta {
            step_id: "s1".into(),
            text: "tok".into(),
        });
        sink.emit(RunEventKind::AssistantMessage {
            step_id: "s1".into(),
            content: "pronto".into(),
            reasoning: String::new(),
        });

        let rows = store.list_run_events("r1", 0).unwrap();
        assert_eq!(rows.len(), 2, "o delta não pode ir ao banco");
        assert_eq!(rows[0].kind, "run.started");
        assert_eq!(rows[1].kind, "assistant.message");
        // O `seq` do banco preserva o buraco deixado pelo delta.
        assert_eq!(rows[1].seq, 3);
        assert_eq!(store.last_run_seq("r1").unwrap(), 3);
    }

    #[test]
    fn attach_replays_from_the_database_and_then_follows_live() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .create_run("r1", None, "m", RunMode::Yolo, true, None, "p")
            .unwrap();
        let sink = EventSink::new("r1", Some(store.clone()));

        sink.emit(started());
        sink.emit(RunEventKind::StepStarted {
            step_id: "s1".into(),
            index: 1,
        });

        // Interface recarregada: reata sem ter visto nada.
        let (cb, seen) = collector();
        sink.attach(0, cb);
        assert_eq!(seen.lock().unwrap().len(), 2, "replay dos dois eventos");

        sink.emit(RunEventKind::RunFinished {
            status: RunStatus::Done,
            summary: "ok".into(),
            usage: UsageStats::default(),
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "o novo evento chega ao vivo");
        assert_eq!(seen[2].seq, 3);
    }

    #[test]
    fn attach_after_a_seq_skips_what_was_already_seen() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .create_run("r1", None, "m", RunMode::Smart, false, None, "p")
            .unwrap();
        let sink = EventSink::new("r1", Some(store.clone()));
        sink.emit(started());
        sink.emit(RunEventKind::StepStarted {
            step_id: "s1".into(),
            index: 1,
        });

        let (cb, seen) = collector();
        sink.attach(1, cb);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].seq, 2);
    }
}
