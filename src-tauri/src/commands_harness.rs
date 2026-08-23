//! "Abrir em um harness": entrega o modelo carregado a um agente de código
//! externo com um clique, no espírito da lista "Applications" do Ollama.
//!
//! O registro é ESTÁTICO e os placeholders (`{baseUrl}`, `{model}`,
//! `{apiKey}`) são resolvidos aqui no backend — a interface nunca monta
//! comando, então nunca vira injeção. A chave de API só viaja por variável de
//! ambiente: argv é visível na lista de processos de qualquer usuário.
//!
//! DeepSeek Harness (`dsh`): o provedor OpenAI-compatible entra por uma seção
//! `llm-pi-ai:` no `settings.yaml` do `DSH_HOME` (fato verificado nos
//! pacotes npm `@deepseek-ai/dsh`/`dsh-base`/`dsh-llm-pi-ai`). Em vez de
//! mesclar YAML no perfil pessoal de quem usa, o app aponta `DSH_HOME` para
//! uma pasta própria (`<data>/dsh-home`) e escreve o arquivo inteiro — sem
//! parser de YAML, sem risco de comer configuração alheia.

use crate::state::AppState;
use serde::Serialize;
use tauri::State;

type CmdResult<T> = Result<T, String>;

/// Variável que carrega a chave de API local até o harness.
const API_KEY_ENV: &str = "OPENWEIGHTS_API_KEY";

/// Um harness externo que o app sabe lançar.
struct HarnessSpec {
    id: &'static str,
    name: &'static str,
    /// Binário sondado com `which`/`where`.
    probe_bin: &'static str,
    /// Comando de instalação mostrado (copiável) quando não instalado.
    install_cmd: &'static str,
    /// Argv de lançamento; aceita placeholders.
    launch: &'static [&'static str],
    /// Variáveis de ambiente; aceitam placeholders. `{apiKey}` que resolver
    /// para vazio não é exportada.
    env: &'static [(&'static str, &'static str)],
    docs_url: &'static str,
    /// O harness cai para `npx <pacote>` quando o binário não está no PATH.
    npx_package: Option<&'static str>,
}

/// O registro. Ordem = ordem dos cartões na tela.
fn registry() -> &'static [HarnessSpec] {
    &[
        HarnessSpec {
            id: "dsh",
            name: "DeepSeek Harness",
            probe_bin: "dsh",
            install_cmd: "npm install -g @deepseek-ai/dsh",
            launch: &["{bin}", "web"],
            // O provedor vem do settings.yaml que o app escreve no DSH_HOME
            // gerenciado; a chave (se houver) vai pela env referenciada lá.
            env: &[("DSH_HOME", "{dshHome}"), (API_KEY_ENV, "{apiKey}")],
            docs_url: "https://github.com/deepseek-ai/deepseek-harness",
            npx_package: Some("@deepseek-ai/dsh"),
        },
        HarnessSpec {
            id: "aider",
            name: "Aider",
            probe_bin: "aider",
            install_cmd: "python -m pip install aider-install && aider-install",
            launch: &[
                "{bin}",
                "--openai-api-base",
                "{baseUrl}",
                "--model",
                "openai/{model}",
            ],
            env: &[("OPENAI_API_KEY", "{apiKeyOrDummy}")],
            docs_url: "https://aider.chat/docs/llms/openai-compat.html",
            npx_package: None,
        },
        HarnessSpec {
            id: "opencode",
            name: "OpenCode",
            probe_bin: "opencode",
            install_cmd: "npm install -g opencode-ai",
            launch: &["{bin}"],
            env: &[
                ("OPENAI_BASE_URL", "{baseUrl}"),
                ("OPENAI_API_KEY", "{apiKeyOrDummy}"),
            ],
            docs_url: "https://opencode.ai/docs",
            npx_package: Some("opencode-ai"),
        },
    ]
}

/// O que a tela mostra por cartão.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// Caminho encontrado, quando instalado.
    pub path: Option<String>,
    /// Dá para abrir mesmo sem instalar (via `npx`).
    pub launchable: bool,
    pub install_cmd: String,
    /// Prévia legível do que o botão "Abrir" executa (segredos mascarados).
    pub command_preview: String,
    pub docs_url: String,
}

/// `which`/`where` sem janela de console.
async fn probe(bin: &str) -> Option<String> {
    let (cmd, arg) = if cfg!(windows) {
        ("where", bin)
    } else {
        ("which", bin)
    };
    let mut c = tokio::process::Command::new(cmd);
    c.arg(arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    lr_proc::no_window(&mut c);
    let out = tokio::time::timeout(std::time::Duration::from_secs(5), c.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

/// Contexto de substituição dos placeholders.
struct Fill {
    base_url: String,
    model: String,
    api_key: Option<String>,
    dsh_home: String,
    bin: String,
}

fn fill(template: &str, f: &Fill, mask_secret: bool) -> String {
    let key = if mask_secret {
        f.api_key.as_ref().map(|_| "•••".to_string())
    } else {
        f.api_key.clone()
    };
    template
        .replace("{baseUrl}", &f.base_url)
        .replace("{model}", &f.model)
        .replace("{apiKey}", key.as_deref().unwrap_or(""))
        // Clientes OpenAI recusam chave vazia mesmo quando o servidor não
        // exige; "local" é o valor de cortesia consagrado.
        .replace("{apiKeyOrDummy}", key.as_deref().unwrap_or("local"))
        .replace("{dshHome}", &f.dsh_home)
        .replace("{bin}", &f.bin)
}

async fn server_fill(state: &AppState, model: &str) -> CmdResult<Fill> {
    let (base_url, api_key) = {
        let guard = state.server.lock().await;
        match guard.as_ref() {
            Some(srv) if srv.is_spawned() => {
                (srv.config().connect_url(), srv.config().api_key.clone())
            }
            _ => return Err("servidor não está rodando".to_string()),
        }
    };
    Ok(Fill {
        base_url: format!("{base_url}/v1"),
        model: model.to_string(),
        api_key,
        dsh_home: state
            .data_dir
            .join("dsh-home")
            .to_string_lossy()
            .into_owned(),
        bin: String::new(),
    })
}

#[tauri::command]
pub async fn harness_list(
    state: State<'_, AppState>,
    model: String,
) -> CmdResult<Vec<HarnessStatus>> {
    let mut f = server_fill(&state, model.trim()).await.unwrap_or(Fill {
        base_url: "http://127.0.0.1:11711/v1".into(),
        model: model.trim().to_string(),
        api_key: None,
        dsh_home: state
            .data_dir
            .join("dsh-home")
            .to_string_lossy()
            .into_owned(),
        bin: String::new(),
    });

    let mut out = Vec::new();
    for spec in registry() {
        let path = probe(spec.probe_bin).await;
        let npx = if path.is_none() && spec.npx_package.is_some() {
            probe("npx").await.is_some()
        } else {
            false
        };
        f.bin = match (&path, spec.npx_package) {
            (Some(_), _) => spec.probe_bin.to_string(),
            (None, Some(pkg)) => format!("npx -y {pkg}"),
            (None, None) => spec.probe_bin.to_string(),
        };
        let argv: Vec<String> = spec.launch.iter().map(|t| fill(t, &f, true)).collect();
        let envs: Vec<String> = spec
            .env
            .iter()
            .filter_map(|(k, t)| {
                let v = fill(t, &f, true);
                (!v.is_empty()).then(|| format!("{k}={v}"))
            })
            .collect();
        let command_preview = format!("{} {}", envs.join(" "), argv.join(" "))
            .trim()
            .to_string();
        out.push(HarnessStatus {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            installed: path.is_some(),
            path,
            launchable: f.bin == spec.probe_bin || npx,
            install_cmd: spec.install_cmd.to_string(),
            command_preview,
            docs_url: spec.docs_url.to_string(),
        });
    }
    Ok(out)
}

/// O `settings.yaml` que faz o modelo local aparecer no seletor do dsh.
///
/// Formato do adaptador `dsh-llm-pi-ai` (rota declarada à mão): `api:
/// openai-completions` + `baseURL` + catálogo de modelos. A chave nunca entra
/// no arquivo — `apiKeyEnv` é uma referência resolvida por requisição.
fn dsh_settings_yaml(f: &Fill, train_ctx: Option<u32>) -> String {
    let ctx = train_ctx.unwrap_or(32_768);
    let mut y = String::from("# gerado pelo OpenWeights — reescrito a cada lançamento\n");
    y.push_str("llm-pi-ai:\n  providers:\n    openweights:\n");
    y.push_str("      displayName: OpenWeights (local)\n");
    y.push_str("      api: openai-completions\n");
    y.push_str(&format!("      baseURL: {}\n", f.base_url));
    if f.api_key.is_some() {
        y.push_str(&format!("      apiKeyEnv: {API_KEY_ENV}\n"));
    }
    y.push_str("      models:\n");
    y.push_str(&format!("        - id: \"{}\"\n", f.model.replace('"', "")));
    y.push_str(&format!(
        "          name: \"{}\"\n",
        f.model.replace('"', "")
    ));
    y.push_str(&format!("          contextWindow: {ctx}\n"));
    y
}

#[tauri::command]
pub async fn harness_launch(
    state: State<'_, AppState>,
    id: String,
    model: String,
    workdir: Option<String>,
) -> CmdResult<()> {
    let spec = registry()
        .iter()
        .find(|s| s.id == id)
        .ok_or("harness desconhecido")?;
    let model = model.trim();
    if model.is_empty() {
        return Err("modelo vazio".into());
    }
    let mut f = server_fill(&state, model).await?;

    let path = probe(spec.probe_bin).await;
    f.bin = match (&path, spec.npx_package) {
        (Some(p), _) => p.clone(),
        (None, Some(pkg)) => {
            probe("npx")
                .await
                .ok_or_else(|| format!("{} não está instalado (nem o npx)", spec.name))?;
            format!("npx -y {pkg}")
        }
        (None, None) => return Err(format!("{} não está instalado", spec.name)),
    };

    // dsh: escrever o provedor no DSH_HOME gerenciado antes de abrir.
    if spec.id == "dsh" {
        let home = state.data_dir.join("dsh-home");
        std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        let ctx = crate::commands::profile_for(&state, model).and_then(|p| p.ctx);
        std::fs::write(home.join("settings.yaml"), dsh_settings_yaml(&f, ctx))
            .map_err(|e| e.to_string())?;
    }

    // Monta o argv real (sem máscara) e o diretório de trabalho.
    let argv: Vec<String> = spec
        .launch
        .iter()
        .flat_map(|t| {
            // `{bin}` pode ter virado "npx -y pacote" — reparte em tokens.
            let cheio = fill(t, &f, false);
            if *t == "{bin}" {
                cheio.split(' ').map(str::to_string).collect::<Vec<_>>()
            } else {
                vec![cheio]
            }
        })
        .collect();
    let cwd = workdir
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Terminal interativo próprio: o harness é um programa de terminal, não
    // um daemon — precisa de console visível e independente do app.
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new("cmd");
        let linha = argv.join(" ");
        // O `cmd /c` de fora fica sem janela (no_window_std); quem abre o
        // console visível — e independente do app — é o `start`.
        c.args(["/c", "start", "", "cmd", "/k", &linha])
            .current_dir(&cwd);
        lr_proc::no_window_std(&mut c);
        for (k, t) in spec.env {
            let v = fill(t, &f, false);
            if !v.is_empty() {
                c.env(k, v);
            }
        }
        c.spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let mut c = std::process::Command::new(&argv[0]);
        c.args(&argv[1..])
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        for (k, t) in spec.env {
            let v = fill(t, &f, false);
            if !v.is_empty() {
                c.env(k, v);
            }
        }
        c.spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contexto() -> Fill {
        Fill {
            base_url: "http://127.0.0.1:11711/v1".into(),
            model: "Qwen3.6-27B-MTP.gguf".into(),
            api_key: Some("sk-segredo".into()),
            dsh_home: "/data/dsh-home".into(),
            bin: "dsh".into(),
        }
    }

    /// O segredo nunca aparece na prévia — e nunca entra no argv.
    #[test]
    fn the_preview_masks_the_secret() {
        let f = contexto();
        assert_eq!(fill("{apiKey}", &f, true), "•••");
        assert_eq!(fill("{apiKey}", &f, false), "sk-segredo");
        assert_eq!(fill("{apiKeyOrDummy}", &f, true), "•••");
        // Sem chave, o dummy entra para clientes que recusam chave vazia.
        let sem = Fill {
            api_key: None,
            ..contexto()
        };
        assert_eq!(fill("{apiKeyOrDummy}", &sem, false), "local");
        assert_eq!(fill("{apiKey}", &sem, false), "");
    }

    #[test]
    fn dsh_settings_declare_the_local_route() {
        let y = dsh_settings_yaml(&contexto(), Some(131_072));
        assert!(y.contains("llm-pi-ai:"));
        assert!(y.contains("api: openai-completions"));
        assert!(y.contains("baseURL: http://127.0.0.1:11711/v1"));
        assert!(y.contains("apiKeyEnv: OPENWEIGHTS_API_KEY"));
        assert!(!y.contains("sk-segredo"), "segredo nunca entra no arquivo");
        assert!(y.contains("contextWindow: 131072"));
        // Sem chave configurada, a rota fica sem autenticação — e sem a
        // referência, que com env ausente derrubaria toda requisição com
        // MISSING_CREDENTIAL.
        let sem = dsh_settings_yaml(
            &Fill {
                api_key: None,
                ..contexto()
            },
            None,
        );
        assert!(!sem.contains("apiKeyEnv"));
        assert!(sem.contains("contextWindow: 32768"));
    }

    #[test]
    fn every_registry_entry_is_consistent() {
        let mut ids = std::collections::HashSet::new();
        for s in registry() {
            assert!(ids.insert(s.id), "id repetido: {}", s.id);
            assert!(!s.launch.is_empty());
            assert!(
                s.launch[0].contains("{bin}"),
                "{}: launch começa no binário",
                s.id
            );
        }
    }
}
