//! O Code Mode dentro do laço do agente.
//!
//! O crate `lr_codemode` sabe gerar o SDK, subir a ponte e rodar o Node.
//! O que mora aqui é o que só o laço pode decidir: como a ferramenta
//! `run_code` se apresenta ao modelo, o que o prompt diz sobre ela, como as
//! chamadas que chegam pela ponte viram chamadas normais do harness, e o que
//! volta para a conversa quando o script termina.
//!
//! ## Um passo, muitas chamadas
//!
//! No modo nativo cada ferramenta gasta um passo: o modelo pede, o servidor
//! reprocessa a conversa inteira, o resultado empilha na janela. Aqui o
//! modelo gasta **um** passo para escrever o programa e o harness executa
//! quantas chamadas o programa fizer, sem que nenhuma delas passe pelo
//! modelo. O que volta é o que o script imprimiu.
//!
//! ## O que não muda
//!
//! Toda chamada vinda do script atravessa o mesmo caminho de uma chamada
//! normal — política, confirmação, foto do projeto, trilha, contadores. É por
//! isso que o despacho é um trait implementado pelo `ToolRunner`, e não uma
//! execução direta contra o registro: um atalho aqui apagaria de uma vez as
//! proteções que o harness levou seis fases para ganhar.

use lr_codemode::bridge::{BridgeRequest, CallReply};
use lr_types::agent::{RunMode, ToolCategory, ToolOrigin, ToolSpec, ToolTier};
use serde_json::{Value, json};
use std::future::Future;

/// Nome da ferramenta que carrega o programa.
pub(crate) const RUN_CODE: &str = "run_code";

/// Como a ferramenta se apresenta ao modelo.
///
/// A descrição não fala de "Code Mode" nem de arquitetura: fala do que fazer.
/// Modelo pequeno segue exemplo, não conceito.
pub(crate) fn spec_run_code(assinaturas: &str) -> ToolSpec {
    ToolSpec {
        name: RUN_CODE.into(),
        description: format!(
            "Executa um programa JavaScript que usa as ferramentas do projeto e devolve o que \
             ele imprimir. Prefira SEMPRE esta ferramenta: um programa resolve a tarefa \
             inteira de uma vez, em vez de uma chamada por passo.\n\nFerramentas disponíveis \
             dentro do programa (todas devolvem texto e precisam de `await`):\n{assinaturas}\n\
             Use `say(...)` para imprimir o resultado: só o que for impresso volta para você."
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "O programa, em JavaScript moderno. `await` no topo é permitido. Termine imprimindo o resultado com say()."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        }),
        category: ToolCategory::Execute,
        tier: ToolTier::Caution,
        origin: ToolOrigin::Builtin,
        read_only: false,
    }
}

/// A seção do prompt de sistema que substitui a lista de ferramentas.
pub(crate) fn secao_do_prompt(assinaturas: &str, mode: RunMode) -> String {
    let mut out = String::with_capacity(assinaturas.len() + 700);
    out.push_str(
        "\n## Como agir: escreva um programa\n\
         Você age chamando `run_code` com um programa JavaScript. Dentro dele, cada ferramenta \
         é uma função assíncrona que devolve texto:\n\n",
    );
    out.push_str(assinaturas);
    out.push_str(
        "\nRegras do programa:\n\
         - Sempre `await` na chamada; `say(...)` imprime, e só o impresso volta para você.\n\
         - Faça a tarefa INTEIRA num programa só: laço, condição e contas resolvem ali \
           dentro, sem gastar um passo por arquivo.\n\
         - Uma ferramenta que falha levanta `ToolError`: trate com `try/catch` e siga com o \
           resto em vez de deixar o programa morrer.\n\
         - Só as funções acima tocam o projeto: `require`, `import` de módulo do \
           Node e `fs` não funcionam — o programa roda isolado.\n\
         - `fs_read` devolve o texto do arquivo; `fs_list`, `fs_glob` e `fs_grep` \
           devolvem ARRAY. As demais devolvem texto.\n\
         - Imprima pouco: o que você imprimir é o que ocupa a sua janela.\n\
         - Falta uma peça que você vai usar muitas vezes? Escreva \
           `.openweights/plugins/<nome>.mjs` com `// @tool {\"name\":\"...\",\
           \"description\":\"...\"}` na primeira linha e \
           `export default async function (args) {...}`; ela vira uma função \
           disponível no próximo programa.\n",
    );
    if mode == RunMode::Approve {
        out.push_str(
            "- A pessoa autoriza o programa uma vez, antes de ele rodar: escreva-o completo, \
             porque pedir de novo custa outra autorização.\n",
        );
    }
    out
}

/// O programa entregue em texto, quando o modelo não emite a chamada.
///
/// Modelo pequeno que não fala o protocolo de tool call escreve o programa
/// num bloco cercado e encerra o passo — o run terminava sem fazer nada. Aqui
/// o bloco vira a chamada `run_code`, que passa pela política como qualquer
/// outra. É o mesmo remédio que o harness já usa para arquivo entregue em
/// texto (`arquivo_em_texto`), e é o que faz um modelo sem tool call nativo
/// trabalhar de verdade.
pub(crate) fn bloco_de_codigo(texto: &str) -> Option<String> {
    // O outro protocolo de texto do harness tem prioridade: `ARQUIVO: x.js`
    // seguido de um bloco cercado é um arquivo para GRAVAR, e o conteúdo dele
    // pode ser JavaScript com `await` — que aqui passaria por programa e
    // seria executado em vez de salvo.
    if texto.contains("ARQUIVO:") {
        return None;
    }
    let mut candidato_sem_idioma = None;

    let mut resto = texto;
    while let Some(inicio) = resto.find("```") {
        let depois = &resto[inicio + 3..];
        let fim_da_linha = depois.find('\n')?;
        let idioma = depois[..fim_da_linha].trim().to_ascii_lowercase();
        let corpo_inicio = fim_da_linha + 1;
        let corpo = &depois[corpo_inicio..];
        let fim = corpo.find("```")?;
        let programa = corpo[..fim].trim_end().to_string();
        resto = &corpo[fim + 3..];

        if programa.trim().is_empty() {
            continue;
        }
        match idioma.as_str() {
            "js" | "javascript" | "mjs" | "node" | "typescript" | "ts" => return Some(programa),
            // Sem idioma só serve se o conteúdo se parecer com um programa
            // nosso: senão um bloco de saída de terminal viraria código.
            "" if parece_programa(&programa) => {
                candidato_sem_idioma.get_or_insert(programa);
            }
            _ => {}
        }
    }
    candidato_sem_idioma
}

fn parece_programa(corpo: &str) -> bool {
    (corpo.contains("await ") || corpo.contains("say(")) && corpo.contains('(')
}

/// Traduz os erros de Node que um modelo pequeno mais comete em instrução.
///
/// O stack trace do Node é preciso e inútil para quem precisa se corrigir:
/// `ReferenceError: require is not defined in ES module scope` não diz o que
/// fazer. Medido com o `qwen2.5-coder:14b`, que começou dois programas
/// seguidos com `require("fs")` mesmo com o prompt avisando — uma linha
/// acionável no RESULTADO vale mais do que outra no prompt, porque chega
/// exatamente no momento do erro.
pub(crate) fn dica_para(erros: &str) -> Option<&'static str> {
    if erros.contains("require is not defined") {
        return Some(
            "Dica: `require` não existe aqui. As ferramentas já são funções globais — \
             chame `await fs_read({path})` direto, sem importar nada.",
        );
    }
    if erros.contains("ERR_ACCESS_DENIED") {
        return Some(
            "Dica: o programa não tem acesso a arquivo por fora das ferramentas. \
             Use `await fs_write({path, content})` em vez de `fs` do Node.",
        );
    }
    if erros.contains("is not a function") {
        return Some("Dica: chame só as funções listadas, com o nome exato, e sempre com `await`.");
    }
    None
}

/// O que o script fez, para o rodapé que volta ao modelo.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Contagem {
    pub chamadas: u32,
    pub falhas: u32,
}

impl Contagem {
    /// Uma linha só: o modelo precisa saber quantas ferramentas rodaram sem
    /// receber o resultado de cada uma — que é justamente a economia.
    pub fn rodape(&self) -> String {
        match (self.chamadas, self.falhas) {
            (0, _) => "\n[o programa não chamou nenhuma ferramenta]".into(),
            (n, 0) => format!("\n[{n} chamadas de ferramenta rodaram dentro do programa]"),
            (n, f) => {
                format!("\n[{n} chamadas de ferramenta rodaram dentro do programa; {f} falharam]")
            }
        }
    }
}

/// A resposta do harness a uma chamada pedida pelo script.
pub(crate) enum Resposta {
    /// Texto para a trilha e, quando a ferramenta tem uma forma, o dado que
    /// o programa recebe de verdade.
    Ok(String, Option<Value>),
    /// Erro que o script pode tratar (política negou, argumento inválido).
    Erro(String),
    /// O run acabou no meio: o script tem que morrer junto.
    Parar(String),
}

/// Quem sabe executar uma chamada com todas as proteções do harness.
#[async_trait::async_trait]
pub(crate) trait Despachante {
    async fn chamar(&mut self, tool: &str, args: Value) -> Resposta;
}

/// Motivo de o atendimento ter parado antes de o script terminar.
pub(crate) type Parada = Option<String>;

/// Atende a ponte enquanto o script roda.
///
/// O `select!` alterna entre "chegou uma chamada" e "o script terminou". Um
/// detalhe importante: enquanto uma chamada está sendo decidida — inclusive
/// esperando uma pessoa autorizar — o futuro do script não avança, e o prazo
/// dele também não corre. Isso é de propósito: ninguém deve perder o trabalho
/// do script porque demorou a clicar em "permitir". O script, do lado dele,
/// está parado no `fetch` esperando a mesma resposta.
pub(crate) async fn hospedar<S, D>(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<BridgeRequest>,
    script: S,
    despachante: &mut D,
) -> (Option<S::Output>, Contagem, Parada)
where
    S: Future,
    D: Despachante,
{
    let mut contagem = Contagem::default();
    tokio::pin!(script);

    loop {
        tokio::select! {
            biased;
            pedido = rx.recv() => {
                let Some(pedido) = pedido else {
                    // A ponte caiu (o `Bridge` foi solto): só resta esperar o
                    // script terminar.
                    return (Some(script.await), contagem, None);
                };
                contagem.chamadas += 1;
                match despachante.chamar(&pedido.tool, pedido.args).await {
                    Resposta::Ok(texto, dados) => {
                        let _ = pedido.reply.send(CallReply::dados(texto, dados));
                    }
                    Resposta::Erro(texto) => {
                        contagem.falhas += 1;
                        let _ = pedido.reply.send(CallReply::err(texto));
                    }
                    Resposta::Parar(motivo) => {
                        contagem.falhas += 1;
                        let _ = pedido.reply.send(CallReply::err(motivo.clone()));
                        // Sair daqui solta o futuro do script, e com ele o
                        // processo: o `spawner` mata a árvore no `Drop`.
                        return (None, contagem, Some(motivo));
                    }
                }
            }
            saida = &mut script => return (Some(saida), contagem, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_codemode::bridge::CallReply;
    use tokio::sync::{mpsc, oneshot};

    #[test]
    fn bloco_com_idioma_javascript_vira_programa() {
        let texto =
            "Vou fazer assim:\n\n```js\nconst a = await fs_read({path:\"a\"});\nsay(a);\n```\n";
        let programa = bloco_de_codigo(texto).expect("achou o bloco");
        assert!(programa.starts_with("const a = await fs_read"));
        assert!(programa.ends_with("say(a);"));
    }

    #[test]
    fn bloco_de_saida_de_terminal_nao_e_confundido_com_programa() {
        let texto = "O resultado foi:\n\n```\ntest result: ok. 12 passed\n```\n";
        assert!(bloco_de_codigo(texto).is_none());
    }

    #[test]
    fn bloco_sem_idioma_que_parece_programa_e_aceito() {
        let texto = "```\nconst t = await fs_read({path:\"a\"});\nsay(t);\n```";
        assert!(bloco_de_codigo(texto).is_some());
    }

    /// Um arquivo JavaScript entregue pelo protocolo de texto continua sendo
    /// um arquivo: gravar e executar não são a mesma coisa.
    #[test]
    fn arquivo_entregue_em_texto_nao_vira_programa() {
        let texto = "ARQUIVO: soma.js\n```js\nconst t = await fs_read({path:\"a\"});\n```\n";
        assert!(bloco_de_codigo(texto).is_none());
    }

    #[test]
    fn bloco_de_outra_linguagem_e_ignorado() {
        let texto = "```python\nprint(await x())\n```";
        assert!(bloco_de_codigo(texto).is_none());
    }

    #[test]
    fn cerca_sem_fechamento_nao_derruba_nem_inventa_programa() {
        assert!(bloco_de_codigo("```js\nsay(1);").is_none());
        assert!(bloco_de_codigo("sem cerca nenhuma").is_none());
    }

    #[test]
    fn a_ferramenta_anuncia_as_assinaturas_e_pede_say() {
        let spec = spec_run_code("await fs_read({ path }) — Lê um arquivo.\n");
        assert_eq!(spec.name, RUN_CODE);
        assert!(spec.description.contains("await fs_read({ path })"));
        assert!(spec.description.contains("say("));
        assert_eq!(spec.category, ToolCategory::Execute);
    }

    #[test]
    fn erro_de_node_vira_instrucao() {
        let erro = "ReferenceError: require is not defined in ES module scope";
        assert!(dica_para(erro).is_some_and(|d| d.contains("funções globais")));
        assert!(dica_para("Error: ERR_ACCESS_DENIED").is_some_and(|d| d.contains("fs_write")));
        assert!(dica_para("tudo certo").is_none());
    }

    #[test]
    fn o_rodape_conta_o_que_o_modelo_nao_viu() {
        assert!(Contagem::default().rodape().contains("não chamou"));
        let c = Contagem {
            chamadas: 12,
            falhas: 0,
        };
        assert!(c.rodape().contains("12 chamadas"));
        let c = Contagem {
            chamadas: 12,
            falhas: 2,
        };
        assert!(c.rodape().contains("2 falharam"));
    }

    struct Falso {
        respostas: Vec<Resposta>,
    }

    #[async_trait::async_trait]
    impl Despachante for Falso {
        async fn chamar(&mut self, _tool: &str, _args: Value) -> Resposta {
            match self.respostas.pop() {
                Some(r) => r,
                None => Resposta::Ok("vazio".into(), None),
            }
        }
    }

    /// Manda uma chamada pela fila e devolve o que o script receberia.
    async fn pedir(tx: &mpsc::UnboundedSender<BridgeRequest>) -> oneshot::Receiver<CallReply> {
        let (reply, rx) = oneshot::channel();
        tx.send(BridgeRequest {
            tool: "fs_read".into(),
            args: json!({"path": "a.txt"}),
            reply,
        })
        .unwrap();
        rx
    }

    #[tokio::test]
    async fn as_chamadas_do_script_sao_atendidas_ate_ele_terminar() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut falso = Falso {
            respostas: vec![Resposta::Ok("conteúdo".into(), None)],
        };

        let (fim_tx, fim_rx) = oneshot::channel::<u8>();
        let script = async move {
            let volta = pedir(&tx).await;
            let resposta = volta.await.unwrap();
            assert!(resposta.ok);
            assert_eq!(resposta.content, "conteúdo");
            fim_tx.send(1).unwrap();
            "terminou"
        };

        let (saida, contagem, parada) = hospedar(rx, script, &mut falso).await;
        assert_eq!(saida, Some("terminou"));
        assert_eq!(contagem.chamadas, 1);
        assert_eq!(contagem.falhas, 0);
        assert!(parada.is_none());
        assert_eq!(fim_rx.await.unwrap(), 1);
    }

    #[tokio::test]
    async fn erro_de_politica_volta_ao_script_sem_derrubar_o_programa() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut falso = Falso {
            respostas: vec![
                Resposta::Ok("segunda".into(), None),
                Resposta::Erro("negado pela política".into()),
            ],
        };

        let script = async move {
            let primeira = pedir(&tx).await.await.unwrap();
            assert!(!primeira.ok);
            assert_eq!(primeira.content, "negado pela política");
            // O programa segue depois do `catch`.
            let segunda = pedir(&tx).await.await.unwrap();
            assert!(segunda.ok);
            "terminou"
        };

        let (saida, contagem, _) = hospedar(rx, script, &mut falso).await;
        assert_eq!(saida, Some("terminou"));
        assert_eq!(contagem.chamadas, 2);
        assert_eq!(contagem.falhas, 1);
    }

    #[tokio::test]
    async fn run_encerrado_no_meio_para_o_atendimento_e_solta_o_script() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut falso = Falso {
            respostas: vec![Resposta::Parar("o run foi cancelado".into())],
        };

        let (marcador_tx, marcador_rx) = oneshot::channel::<u8>();
        let script = async move {
            let _ = pedir(&tx).await.await;
            // Não deve chegar aqui: o futuro é solto assim que paramos.
            marcador_tx.send(1).unwrap();
            "terminou"
        };

        let (saida, contagem, parada) = hospedar(rx, script, &mut falso).await;
        assert_eq!(saida, None);
        assert_eq!(parada.as_deref(), Some("o run foi cancelado"));
        assert_eq!(contagem.falhas, 1);
        assert!(marcador_rx.await.is_err(), "o script devia ter sido solto");
    }
}
