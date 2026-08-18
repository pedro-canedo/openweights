//! Code Mode: o modelo escreve **um programa** que usa as ferramentas.
//!
//! No modo nativo cada ferramenta é uma ida e volta ao modelo, e tudo que
//! voltou fica no histórico para sempre: ler doze arquivos custa doze rodadas
//! de processamento e doze conteúdos empilhados na janela. Aqui o modelo
//! recebe as ferramentas como uma biblioteca, escreve um script que as
//! combina, e só o que o script imprime volta para a conversa.
//!
//! Para modelos pequenos isto vale mais do que velocidade. Eles leram milhões
//! de linhas de código de verdade e pouquíssimos exemplos de preencher
//! formulário JSON — nesta máquina o `qwen2.5-coder:14b` termina a tarefa mas
//! escreve a chamada em texto em vez de emiti-la pelo protocolo. Escrever um
//! programa é o que eles fazem bem.
//!
//! ## As três peças
//!
//! - [`sdk`] gera, a partir dos [`ToolSpec`](lr_types::agent::ToolSpec) do
//!   run, o módulo JavaScript que o script importa e as assinaturas que vão
//!   no prompt.
//! - [`bridge`] sobe um servidor de uma linha só em `127.0.0.1`, com token,
//!   por onde o script pede cada ferramenta de volta ao harness.
//! - [`exec`] escreve os arquivos, roda o Node com prazo e mata a árvore.
//!
//! ## O que este crate NÃO faz
//!
//! Ele não decide se uma chamada pode acontecer. Quem responde às
//! requisições da ponte é o laço do agente, que passa cada uma pela política,
//! pela foto do projeto e pela trilha — os mesmos caminhos de uma chamada
//! normal. Um atalho aqui seria uma porta dos fundos para tudo que o harness
//! aprendeu a proteger.

pub mod bridge;
pub mod exec;
pub mod sdk;

pub use bridge::{Bridge, BridgeRequest, CallReply};
pub use exec::{ScriptOutcome, ScriptRequest, node_program, run_script};
pub use sdk::{render_module, render_signatures, safe_ident};
