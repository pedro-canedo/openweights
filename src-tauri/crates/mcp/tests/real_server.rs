//! Teste de fumaça contra um servidor MCP de verdade.
//!
//! Fica `#[ignore]` de propósito: depende de rede, de `npx` e de baixar um
//! pacote — nada disso pode entrar no `cargo test` do dia a dia. Serve para
//! conferir à mão que o handshake, a listagem e a chamada funcionam contra
//! uma implementação real do protocolo (e, no Windows, que o `cmd /c` de
//! fato resolve o `npx.cmd`).
//!
//! Como rodar:
//!
//! ```text
//! cargo test -p lr_mcp --test real_server -- --ignored --nocapture
//! ```

use lr_mcp::{McpHost, config};
use std::sync::Arc;

/// Servidor de referência do próprio projeto MCP.
const EVERYTHING: &str = r#"{
  "mcpServers": {
    "everything": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-everything"]
    }
  }
}"#;

#[tokio::test]
#[ignore = "precisa de rede e de npx instalado"]
async fn connects_lists_and_calls_a_real_stdio_server() {
    let store = Arc::new(lr_store::Store::open_in_memory().unwrap());
    let host = McpHost::new(store.clone());
    host.add("everything", EVERYTHING).await.unwrap();

    let tools = host
        .refresh("everything")
        .await
        .expect("o servidor de referência deveria conectar");
    assert!(!tools.is_empty(), "o catálogo veio vazio");
    println!("{} ferramentas:", tools.len());
    for tool in &tools {
        println!("  {} [{:?}/{:?}]", tool.name, tool.category(), tool.tier());
    }

    // Antes da aprovação, nada é exposto — o portão vale também aqui.
    assert!(host.exposed_specs("everything").await.is_empty());

    let hash = store
        .get_mcp_server("everything")
        .unwrap()
        .unwrap()
        .tools_hash
        .unwrap();
    host.approve_tools("everything", &hash).await.unwrap();
    assert!(!host.exposed_specs("everything").await.is_empty());

    // `echo` faz parte do servidor de referência.
    let out = host
        .call("everything", "echo", serde_json::json!({"message": "olá"}))
        .await
        .expect("a chamada deveria funcionar");
    assert!(out.text.contains("olá"), "resposta: {}", out.text);

    // Reconectar tem de dar o mesmo hash, senão o app pediria re-aprovação a
    // cada abertura do programa.
    host.disconnect("everything").await;
    host.refresh("everything").await.unwrap();
    let again = store
        .get_mcp_server("everything")
        .unwrap()
        .unwrap()
        .tools_hash
        .unwrap();
    assert_eq!(hash, again, "o hash tem de ser estável entre conexões");

    host.shutdown().await;
}

#[test]
fn windows_launch_rule_matches_what_the_real_server_needs() {
    // Não precisa de rede: confere que o comando do JSON acima vira
    // `cmd /c npx ...` no Windows e continua `npx ...` fora dele.
    let cfg = config::parse_servers(EVERYTHING).unwrap().remove(0);
    let (command, args) = match &cfg.transport {
        lr_mcp::McpTransport::Stdio { command, args, .. } => (command.clone(), args.clone()),
        _ => unreachable!(),
    };
    let (prog, out) = config::normalize_launch(&command, &args, true);
    assert_eq!(prog, "cmd");
    assert_eq!(out[0], "/c");
    assert_eq!(out[1], "npx");

    let (prog, out) = config::normalize_launch(&command, &args, false);
    assert_eq!(prog, "npx");
    assert_eq!(out, args);
}
