//! Caminho vetorial ponta a ponta, contra um **servidor de embeddings falso**
//! (`std::net`, sem dependência nova).
//!
//! Por que um servidor falso e não um mock do cliente: o que precisa ser
//! provado é justamente a integração — que o vetor sai do `/v1/embeddings`,
//! entra na tabela `vec0` com a dimensão certa e volta na fusão. Um mock do
//! `LlamaClient` testaria o mock.
//!
//! O "modelo" falso é um saco de conceitos: cada dimensão conta as palavras de
//! um campo semântico. Isso dá o comportamento que interessa ao teste — texto
//! e consulta SEM nenhuma palavra em comum ainda assim se aproximam no espaço
//! vetorial, que é exatamente o que o BM25 não consegue fazer.

use lr_rag::index::{IndexOptions, index_workspace};
use lr_rag::search::{HitSource, search};
use lr_rag::{EmbedConfig, EmbedEndpoint, RagHandle};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Campos semânticos do "modelo" falso. Uma dimensão por campo.
const CONCEPTS: [&[&str]; 3] = [
    &["ttl", "expira", "prazo", "duracao", "tempo", "segundos"],
    &["azul", "verde", "cor", "tinta", "paleta"],
    &["http", "socket", "porta", "rede", "conexao"],
];

fn fake_embed(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    let mut v = vec![0.0f32; CONCEPTS.len() + 1];
    for (i, words) in CONCEPTS.iter().enumerate() {
        for w in *words {
            if lower.contains(w) {
                v[i] += 1.0;
            }
        }
    }
    // Dimensão de fundo: evita vetor todo zero (que normalizaria para nada).
    v[CONCEPTS.len()] = 0.1;
    v
}

struct FakeEmbedServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl FakeEmbedServer {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let (flag, counter) = (stop.clone(), calls.clone());

        std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if let Some((path, body)) = read_request(&mut stream) {
                            counter.fetch_add(1, Ordering::SeqCst);
                            let res = if path == "/v1/embeddings" {
                                embeddings_response(&body)
                            } else {
                                "{}".to_string()
                            };
                            write_response(&mut stream, &res);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self { addr, stop, calls }
    }

    fn endpoint(&self) -> EmbedEndpoint {
        EmbedEndpoint {
            base_url: format!("http://{}", self.addr),
            api_key: None,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for FakeEmbedServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn embeddings_response(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let inputs: Vec<String> = v["input"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    let data: Vec<serde_json::Value> = inputs
        .iter()
        .enumerate()
        .map(|(i, text)| serde_json::json!({ "index": i, "embedding": fake_embed(text) }))
        .collect();
    serde_json::json!({ "object": "list", "data": data }).to_string()
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.lines();
    let path = lines.next()?.split_whitespace().nth(1)?.to_string();
    let mut len = 0usize;
    for line in lines {
        if let Some((k, val)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = val.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    if len > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    Some((path, String::from_utf8_lossy(&body).to_string()))
}

fn write_response(stream: &mut TcpStream, body: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

// ------------------------------------------------------------------ testes --

fn sample_project(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/sessao.rs"),
        "// quanto tempo a sessao dura antes de cair\npub const TTL_SEGUNDOS: u64 = 3600;\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/tema.rs"),
        "// paleta padrao da interface\npub const COR_PRIMARIA: &str = \"azul\";\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/rede.rs"),
        "// abre a conexao http na porta configurada\npub fn abrir_socket() {}\n",
    )
    .unwrap();
}

fn embed_cfg(server: &FakeEmbedServer) -> EmbedConfig {
    EmbedConfig {
        model: Some("fake-embed".into()),
        endpoint: Some(server.endpoint()),
    }
}

#[tokio::test]
async fn vectors_are_stored_and_used_in_the_fusion() {
    let server = FakeEmbedServer::spawn();
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let db = dir.path().join("idx.db");

    let report = index_workspace(
        &db,
        &proj,
        IndexOptions {
            embed: embed_cfg(&server),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(report.vector, "o servidor respondeu: deveria ter vetor");
    assert_eq!(report.indexed, 3);
    assert_eq!(report.embedded, report.chunks, "todo trecho ganha vetor");
    assert!(
        server.calls() >= 2,
        "sondagem de dimensão + lote de trechos"
    );

    // A dimensão gravada é a que o servidor devolveu (4 = 3 conceitos + fundo).
    let conn = lr_rag::open_rag_connection(&db).unwrap();
    lr_rag::ensure_schema(&conn).unwrap();
    let dim: String = conn
        .query_row(
            "SELECT value FROM rag_meta WHERE key = 'embed_dim'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dim, "4");
    let stored: i64 = conn
        .query_row("SELECT COUNT(*) FROM rag_vec", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored as usize, report.chunks);
}

/// O caso que justifica o RAG: a consulta não tem NENHUMA palavra em comum com
/// o trecho certo, então o BM25 não tem como achar — só o vetor acha.
#[tokio::test]
async fn semantic_query_without_shared_words_still_finds_the_chunk() {
    let server = FakeEmbedServer::spawn();
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let db = dir.path().join("idx.db");

    index_workspace(
        &db,
        &proj,
        IndexOptions {
            embed: embed_cfg(&server),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // "prazo" e "duracao" não aparecem em nenhum arquivo.
    let hits = search(&db, &proj, "prazo de duracao", 3, embed_cfg(&server))
        .await
        .unwrap();

    assert!(!hits.is_empty(), "o vetor deveria ter achado algo");
    assert_eq!(hits[0].path, "src/sessao.rs");
    assert_eq!(
        hits[0].source,
        HitSource::Vector,
        "sem palavra em comum, o acerto só pode ter vindo do vetor"
    );
    assert!(hits[0].start_line >= 1 && hits[0].end_line >= hits[0].start_line);
}

/// Termo literal presente no arquivo: as duas listas concordam e o trecho sobe.
#[tokio::test]
async fn literal_and_semantic_agree_on_the_same_chunk() {
    let server = FakeEmbedServer::spawn();
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let db = dir.path().join("idx.db");

    index_workspace(
        &db,
        &proj,
        IndexOptions {
            embed: embed_cfg(&server),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let hits = search(&db, &proj, "porta http conexao", 3, embed_cfg(&server))
        .await
        .unwrap();
    assert_eq!(hits[0].path, "src/rede.rs");
    assert_eq!(hits[0].source, HitSource::Both);
}

/// Servidor fora do ar no meio do caminho: a indexação não pode falhar, só
/// perder a parte vetorial. É o degrade gracioso visto de fora.
#[tokio::test]
async fn index_survives_an_embedding_server_that_is_down() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let db = dir.path().join("idx.db");

    let report = index_workspace(
        &db,
        &proj,
        IndexOptions {
            embed: EmbedConfig {
                model: Some("fake-embed".into()),
                // Porta fechada de propósito.
                endpoint: Some(EmbedEndpoint {
                    base_url: "http://127.0.0.1:1".into(),
                    api_key: None,
                }),
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(!report.vector, "sem servidor, sem vetor");
    assert_eq!(report.indexed, 3, "o texto tem que ter entrado assim mesmo");
    assert_eq!(report.embedded, 0);

    // E a busca textual continua respondendo.
    let hits = search(&db, &proj, "paleta", 3, EmbedConfig::default())
        .await
        .unwrap();
    assert_eq!(hits[0].path, "src/tema.rs");
}

/// Trocar o modelo de embedding invalida os vetores antigos: a tabela é
/// recriada e tudo é reembeddado (espaços vetoriais não se misturam).
#[tokio::test]
async fn switching_the_embedding_model_rebuilds_the_vectors() {
    let server = FakeEmbedServer::spawn();
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let db = dir.path().join("idx.db");

    let first = index_workspace(
        &db,
        &proj,
        IndexOptions {
            embed: embed_cfg(&server),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(first.embedded > 0);

    let mut other = embed_cfg(&server);
    other.model = Some("outro-embed".into());
    let second = index_workspace(
        &db,
        &proj,
        IndexOptions {
            embed: other,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(second.indexed, 0, "nenhum arquivo mudou");
    assert_eq!(
        second.embedded, first.embedded,
        "modelo novo obriga a refazer todos os vetores"
    );
}

/// O `RagHandle` é a porta que os comandos e a ferramenta usam.
#[tokio::test]
async fn handle_drives_index_status_search_and_clear() {
    let server = FakeEmbedServer::spawn();
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let handle = RagHandle::new(dir.path().join("idx.db"));
    handle.set_endpoint(Some(server.endpoint()));
    handle.set_embed_model(Some("fake-embed".into()));

    let before = handle.status(&proj).unwrap();
    assert!(!before.indexed);
    assert!(before.capabilities.vector, "sqlite-vec tem que estar vivo");
    assert!(before.capabilities.fts);
    assert!(before.embed_model_configured);

    let report = handle.index(&proj, None).await.unwrap();
    assert_eq!(report.indexed, 3);

    let after = handle.status(&proj).unwrap();
    assert!(after.indexed);
    assert_eq!(after.files, 3);
    assert_eq!(after.vectors, after.chunks);
    assert_eq!(after.embed_model.as_deref(), Some("fake-embed"));
    assert!(!after.indexing);

    let hits = handle.search(&proj, "paleta de cor", 3).await.unwrap();
    assert_eq!(hits[0].path, "src/tema.rs");

    handle.clear(&proj).unwrap();
    let cleared = handle.status(&proj).unwrap();
    assert!(!cleared.indexed);
    assert_eq!(cleared.chunks, 0);
    assert!(handle.search(&proj, "paleta", 3).await.unwrap().is_empty());
}

/// Um pedido de cancelamento antigo não pode derrubar a rodada seguinte: o
/// sinal é zerado no começo de cada indexação. E a trava de "uma de cada vez"
/// tem que ser liberada ao fim, inclusive depois de um cancelamento.
#[tokio::test]
async fn a_stale_cancel_does_not_poison_the_next_run() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    sample_project(&proj);
    let handle = Arc::new(RagHandle::new(dir.path().join("idx.db")));

    handle.cancel();
    let report = handle.index(&proj, None).await.unwrap();
    assert!(
        !report.cancelled,
        "cancelamento velho vazou para a rodada nova"
    );
    assert_eq!(report.indexed, 3);
    assert!(!handle.is_indexing(), "a trava tem que ser liberada");

    let hits = handle.search(&proj, "paleta", 3).await.unwrap();
    assert_eq!(hits[0].path, "src/tema.rs");
}
