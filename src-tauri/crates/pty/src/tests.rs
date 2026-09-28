//! Testes com processos de verdade num pseudoterminal de verdade (Unix).

use super::*;
use std::sync::mpsc;

/// Destino de teste: junta os blocos; `confirma` diz se ele "desenha" (e
/// confirma sozinho) ou se é uma tela travada.
struct Coletor {
    blocos: Arc<Mutex<Vec<Bloco>>>,
}

impl Destino for Coletor {
    fn enviar(&self, bloco: Bloco) -> bool {
        trava(&self.blocos).push(bloco);
        true
    }
}

fn gerente() -> (Gerente, mpsc::Receiver<(SessaoId, Aviso)>) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let g = Gerente::new(Arc::new(move |id, aviso| {
        let _ = trava(&tx).send((id, aviso));
    }));
    (g, rx)
}

fn sh(script: &str) -> Pedido {
    Pedido {
        programa: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        pasta: std::env::temp_dir(),
        env: vec![("OW_TESTE".into(), "sim".into())],
        colunas: 80,
        linhas: 24,
        titulo: "teste".into(),
        tipo: Tipo::OwCli,
    }
}

fn esperar_saida(
    rx: &mpsc::Receiver<(SessaoId, Aviso)>,
    id: SessaoId,
) -> (Vec<Aviso>, Option<i32>) {
    let mut avisos = Vec::new();
    let limite = Instant::now() + Duration::from_secs(20);
    while Instant::now() < limite {
        if let Ok((de, aviso)) = rx.recv_timeout(Duration::from_millis(200)) {
            if de != id {
                continue;
            }
            if let Aviso::Saiu { codigo } = aviso {
                return (avisos, codigo);
            }
            avisos.push(aviso);
        }
    }
    panic!("a sessão {id} não saiu em 20 s");
}

fn texto(blocos: &[Bloco]) -> String {
    String::from_utf8_lossy(
        &blocos
            .iter()
            .flat_map(|b| b.dados.clone())
            .collect::<Vec<_>>(),
    )
    .into_owned()
}

#[test]
fn le_a_saida_e_os_sinais_de_titulo_e_atencao() {
    let (g, rx) = gerente();
    let id = g
        .abrir(sh(
            r#"printf 'ola\n'; printf '\033]0;meu titulo\007'; printf '\033]9;Aprovar?\007'"#,
        ))
        .unwrap();
    let (avisos, codigo) = esperar_saida(&rx, id);
    assert_eq!(codigo, Some(0));
    assert!(
        avisos.contains(&Aviso::Titulo {
            titulo: "meu titulo".into()
        }),
        "{avisos:?}"
    );
    assert!(
        avisos.contains(&Aviso::Atencao {
            texto: "Aprovar?".into()
        }),
        "{avisos:?}"
    );

    let blocos = Arc::new(Mutex::new(Vec::new()));
    assert!(g.anexar(
        id,
        None,
        Box::new(Coletor {
            blocos: Arc::clone(&blocos)
        })
    ));
    assert!(texto(&trava(&blocos)).contains("ola"));

    let r = g.listar().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!(
        (r.titulo.as_str(), r.atencao, r.viva),
        ("meu titulo", true, false)
    );
    g.visto(id);
    assert!(!g.listar()[0].atencao);
}

#[test]
fn replay_desde_um_endereco_nao_repete_nem_pula() {
    let (g, rx) = gerente();
    let id = g
        .abrir(sh(
            "i=0; while [ $i -lt 300 ]; do echo linha $i; i=$((i+1)); done",
        ))
        .unwrap();
    esperar_saida(&rx, id);

    let tudo = Arc::new(Mutex::new(Vec::new()));
    g.anexar(
        id,
        None,
        Box::new(Coletor {
            blocos: Arc::clone(&tudo),
        }),
    );
    let completo: Vec<u8> = trava(&tudo).iter().flat_map(|b| b.dados.clone()).collect();

    let meio = (completo.len() / 2) as u64;
    let resto = Arc::new(Mutex::new(Vec::new()));
    g.anexar(
        id,
        Some(meio),
        Box::new(Coletor {
            blocos: Arc::clone(&resto),
        }),
    );
    let b = &trava(&resto)[0];
    assert_eq!(b.offset, meio);
    assert!(!b.truncado);
    assert_eq!(b.dados, completo[meio as usize..]);
}

#[test]
fn tela_travada_segura_o_programa_e_confirmar_solta() {
    let (g, rx) = gerente();
    // 5 MB de saída: com a tela sem confirmar, o leitor tem de parar perto
    // de ALTA em vez de engolir tudo.
    let id = g
        .abrir(sh("head -c 5000000 /dev/zero | tr '\\0' 'x'; echo FIM"))
        .unwrap();
    let blocos = Arc::new(Mutex::new(Vec::new()));
    g.anexar(
        id,
        Some(0),
        Box::new(Coletor {
            blocos: Arc::clone(&blocos),
        }),
    );
    std::thread::sleep(Duration::from_millis(800));
    let parado = g.listar()[0].fim;
    assert!(
        parado <= ALTA + 128 * 1024,
        "leu {parado} bytes sem confirmação"
    );
    assert!(g.listar()[0].viva, "o programa não podia ter terminado");

    // A tela alcança e vai confirmando: o programa termina.
    let limite = Instant::now() + Duration::from_secs(20);
    while g.listar()[0].viva && Instant::now() < limite {
        g.confirmar(id, g.listar()[0].fim);
        std::thread::sleep(Duration::from_millis(5));
    }
    let (_, codigo) = esperar_saida(&rx, id);
    assert_eq!(codigo, Some(0));
    assert!(g.listar()[0].fim >= 5_000_000);
}

#[test]
fn sem_tela_anexada_o_programa_corre_solto() {
    let (g, rx) = gerente();
    let id = g
        .abrir(sh("head -c 3000000 /dev/zero | tr '\\0' 'y'"))
        .unwrap();
    let (_, codigo) = esperar_saida(&rx, id);
    assert_eq!(codigo, Some(0));
    // O anel guardou só o fim, e quem anexar agora sabe que perdeu o começo.
    let blocos = Arc::new(Mutex::new(Vec::new()));
    g.anexar(
        id,
        Some(0),
        Box::new(Coletor {
            blocos: Arc::clone(&blocos),
        }),
    );
    let b = &trava(&blocos)[0];
    assert!(b.truncado);
    assert_eq!(b.dados.len(), CAPACIDADE_DO_ANEL);
}

#[test]
fn escrever_chega_ao_programa() {
    let (g, rx) = gerente();
    let id = g.abrir(sh("read linha; echo \"recebi:$linha\"")).unwrap();
    let blocos = Arc::new(Mutex::new(Vec::new()));
    g.anexar(
        id,
        None,
        Box::new(Coletor {
            blocos: Arc::clone(&blocos),
        }),
    );
    g.escrever(id, b"abacaxi\r").unwrap();
    esperar_saida(&rx, id);
    assert!(
        texto(&trava(&blocos)).contains("recebi:abacaxi"),
        "{}",
        texto(&trava(&blocos))
    );
}

#[test]
fn o_filho_recebe_o_ambiente_do_sistema_e_o_do_pedido() {
    let (g, rx) = gerente();
    let id = g.abrir(sh("echo \"[$TERM][$OW_TESTE][$(pwd)]\"")).unwrap();
    esperar_saida(&rx, id);
    let blocos = Arc::new(Mutex::new(Vec::new()));
    g.anexar(
        id,
        None,
        Box::new(Coletor {
            blocos: Arc::clone(&blocos),
        }),
    );
    let pasta = std::env::temp_dir().canonicalize().unwrap();
    let saida = texto(&trava(&blocos));
    assert!(
        saida.contains(&format!("[xterm-256color][sim][{}]", pasta.display())),
        "{saida}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn encerrar_todas_nao_deixa_orfao_nem_job_em_outro_grupo() {
    let (g, _rx) = gerente();
    // Um job em segundo plano que ignora SIGHUP (como `nohup`), num grupo
    // próprio (`set -m`): o `kill(-pid)` do shell não o alcançaria.
    let mut pedido = sh("set -m; trap '' HUP; (trap '' HUP; exec sleep 1000) & sleep 1000");
    pedido.tipo = Tipo::Shell;
    let id = g.abrir(pedido).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let sid = g
        .listar()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .pid
        .unwrap();
    let antes = encerrar::membros_da_sessao(sid);
    assert!(
        antes.len() >= 2,
        "a sessão devia ter o shell e os sleeps: {antes:?}"
    );

    g.encerrar_todas(Duration::from_millis(300));
    std::thread::sleep(Duration::from_millis(300));
    let vivos: Vec<u32> = antes
        .into_iter()
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|s| !s.contains(") Z "))
                .unwrap_or(false)
        })
        .collect();
    assert!(vivos.is_empty(), "sobraram vivos: {vivos:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn fechar_a_aba_encerra_o_shell() {
    let (g, rx) = gerente();
    let mut pedido = sh("sleep 1000");
    pedido.tipo = Tipo::Shell;
    let id = g.abrir(pedido).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    g.fechar(id);
    let (_, _codigo) = esperar_saida(&rx, id);
    assert!(g.listar().iter().all(|r| r.id != id));
}
