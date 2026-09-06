//! O cartão do modelo virando Markdown que a tela consegue ler.
//!
//! O README do Hub é Markdown *misturado com HTML*: o autor abre com um
//! `<div>` de logotipos, empilha `<img>` de badge, escreve a comparação de
//! benchmark numa `<table>` e volta ao Markdown no meio do arquivo. O
//! renderizador da tela é um renderizador de Markdown — o HTML bruto chega
//! nele como texto e a descrição do modelo aparece na interface como uma
//! parede de `<div style="display: flex; gap: 5px">`.
//!
//! Aqui o HTML é traduzido para o Markdown equivalente antes de sair do
//! Rust: link vira `[texto](url)`, imagem vira `![alt](src)`, `<table>`
//! vira tabela GFM (com a linha separadora que o GFM exige), `<pre>` vira
//! bloco cercado. O que não tem equivalente — `<script>`, `<style>`, o
//! `style=` de cada `<div>` — simplesmente não chega à tela.
//!
//! Duas garantias sustentam o resto do módulo:
//!
//! 1. **Markdown puro sai como entrou.** Só um `<` seguido de nome de tag
//!    conhecido é tratado como tag; `<https://exemplo>` e `<think>` são
//!    texto. Blocos cercados e trechos entre crases passam intocados — um
//!    README que ensina a rodar `llama-server` não pode ter o exemplo
//!    reescrito.
//! 2. **Nada de HTML sobra.** A tela nunca recebe marcação para interpretar,
//!    então não há caminho para o cartão de um autor qualquer injetar
//!    marcação na interface.

/// As tags que reconhecemos. Fora desta lista, `<` é texto — é o que
/// mantém `<https://unsloth.ai>` e `<think>` inteiros na tela.
const TAGS: &[&str] = &[
    "a",
    "abbr",
    "article",
    "aside",
    "b",
    "blockquote",
    "br",
    "button",
    "center",
    "code",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "i",
    "iframe",
    "img",
    "ins",
    "kbd",
    "label",
    "li",
    "main",
    "mark",
    "nav",
    "noscript",
    "ol",
    "p",
    "picture",
    "pre",
    "s",
    "script",
    "section",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
    "video",
];

/// Tags cujo conteúdo é descartado junto com elas: nada ali vira texto útil.
const OPACAS: &[&str] = &["script", "style", "svg", "iframe", "noscript", "video"];

/// Traduz o README para Markdown puro.
///
/// Blocos cercados (``` e ~~~) atravessam sem serem lidos: dentro deles
/// `<int>` é código, não tag.
pub fn card_em_markdown(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len());
    let mut fora = String::new();
    let mut cerca: Option<&'static str> = None;

    for linha in texto.lines() {
        match (cerca, marcador_de_cerca(linha)) {
            // Abre bloco cercado: o que estava pendente vira Markdown antes.
            (None, Some(m)) => {
                saida.push_str(&converte(&fora));
                fora.clear();
                saida.push_str(linha);
                saida.push('\n');
                cerca = Some(m);
            }
            // Fecha com o mesmo marcador que abriu.
            (Some(aberto), Some(m)) if m == aberto => {
                saida.push_str(linha);
                saida.push('\n');
                cerca = None;
            }
            (Some(_), _) => {
                saida.push_str(linha);
                saida.push('\n');
            }
            (None, None) => {
                fora.push_str(linha);
                fora.push('\n');
            }
        }
    }
    saida.push_str(&converte(&fora));
    normaliza(&saida)
}

/// `Some("```")` para a linha que abre ou fecha um bloco cercado.
fn marcador_de_cerca(linha: &str) -> Option<&'static str> {
    let s = linha.trim_start_matches(' ');
    if linha.len() - s.len() > 3 {
        return None; // 4 espaços já é bloco indentado, não cerca
    }
    if s.starts_with("```") {
        Some("```")
    } else if s.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// Uma tabela em construção. O GFM exige a linha `| --- |` logo após o
/// cabeçalho, e o HTML não a tem — ela é emitida ao fechar a primeira `<tr>`,
/// quando enfim se sabe quantas colunas existem.
struct Tabela {
    linhas: usize,
    colunas: usize,
}

/// O texto que está sendo montado, mais o pouco de contexto que muda o que
/// um separador significa: dentro de uma célula ou de um link, um `<br>` que
/// virasse `\n` quebraria a tabela ou o link ao redor dele.
struct Saida {
    txt: String,
    tabela: Option<Tabela>,
    /// Pilha de `<a>` abertos: onde o texto do link começa e para onde ele
    /// aponta. Um link sem href não abre colchete nenhum.
    links: Vec<(usize, Option<String>)>,
    /// A última coisa escrita foi uma quebra vinda de uma tag. O que vier de
    /// espaço logo depois é a indentação do HTML — e quatro espaços depois de
    /// uma linha em branco, em Markdown, são um bloco de código.
    apos_quebra: bool,
}

impl Saida {
    fn new(capacidade: usize) -> Self {
        Self {
            txt: String::with_capacity(capacidade),
            tabela: None,
            links: Vec::new(),
            apos_quebra: false,
        }
    }

    fn em_linha(&self) -> bool {
        self.tabela.is_some() || !self.links.is_empty()
    }

    fn push(&mut self, s: &str) {
        self.txt.push_str(s);
        self.apos_quebra = false;
    }

    /// Separação de parágrafo — ou um simples espaço, onde a quebra
    /// estragaria a estrutura ao redor.
    fn bloco(&mut self) {
        if self.em_linha() {
            self.espaco();
        } else {
            self.quebra_dupla();
        }
    }

    fn quebra_dupla(&mut self) {
        while self.txt.ends_with(' ') || self.txt.ends_with('\t') {
            self.txt.pop();
        }
        if !self.txt.is_empty() && !self.txt.ends_with("\n\n") {
            if self.txt.ends_with('\n') {
                self.txt.push('\n');
            } else {
                self.txt.push_str("\n\n");
            }
        }
        self.apos_quebra = true;
    }

    fn quebra(&mut self) {
        if self.em_linha() {
            self.espaco();
        } else if !self.txt.is_empty() && !self.txt.ends_with('\n') {
            self.txt.push('\n');
            self.apos_quebra = true;
        }
    }

    fn espaco(&mut self) {
        if !self.txt.is_empty() && !self.txt.ends_with(char::is_whitespace) {
            self.txt.push(' ');
        }
    }
}

/// O tradutor propriamente dito, sobre um trecho já sabidamente fora de
/// blocos cercados.
fn converte(entrada: &str) -> String {
    // Cópia em minúsculas ASCII: mesmos comprimentos em bytes, então os
    // índices servem para as duas — é o que permite procurar `</pre` sem
    // perder a caixa original de uma URL.
    let baixo = entrada.to_ascii_lowercase();
    let mut s = Saida::new(entrada.len());
    let mut i = 0;

    while i < entrada.len() {
        // Salta direto para o próximo caractere que pode significar algo.
        let prox = entrada[i..]
            .find(['<', '&', '`'])
            .map(|p| i + p)
            .unwrap_or(entrada.len());
        if prox > i {
            let trecho = &entrada[i..prox];
            // Depois de uma quebra emitida por uma tag, o espaço que o autor
            // usou para indentar o HTML não é texto.
            let trecho = if s.apos_quebra {
                trecho.trim_start()
            } else {
                trecho
            };
            if !trecho.is_empty() {
                s.push(trecho);
            }
            i = prox;
            if i >= entrada.len() {
                break;
            }
        }
        match entrada.as_bytes()[i] {
            b'`' => i = copia_codigo_inline(entrada, i, &mut s),
            b'&' => {
                let (texto, fim) = entidade(entrada, i);
                s.push(&texto);
                i = fim;
            }
            _ => i = aplica_tag(entrada, &baixo, i, &mut s),
        }
    }
    // Um `<a>` que nunca fechou não deve deixar o `[` para trás.
    while let Some((pos, href)) = s.links.pop() {
        if href.is_some() && pos < s.txt.len() {
            s.txt.insert(pos, ' ');
            s.txt.remove(pos + 1);
        }
    }
    s.txt
}

/// Trecho entre crases: copiado como está, com as crases.
fn copia_codigo_inline(entrada: &str, i: usize, s: &mut Saida) -> usize {
    let b = entrada.as_bytes();
    let mut n = 0;
    while i + n < b.len() && b[i + n] == b'`' {
        n += 1;
    }
    let marca = &entrada[i..i + n];
    match entrada[i + n..].find(marca) {
        Some(p) => {
            let fim = i + n + p + n;
            s.push(&entrada[i..fim]);
            fim
        }
        None => {
            s.push(marca);
            i + n
        }
    }
}

/// Lê a tag em `i` e escreve o Markdown equivalente. Devolve por onde
/// continuar — se ali não havia tag conhecida, o `<` é texto.
fn aplica_tag(entrada: &str, baixo: &str, i: usize, s: &mut Saida) -> usize {
    // Comentário HTML: some inteiro.
    if baixo[i..].starts_with("<!--") {
        return match baixo[i + 4..].find("-->") {
            Some(p) => i + 4 + p + 3,
            None => entrada.len(),
        };
    }
    let Some(tag) = le_tag(entrada, baixo, i) else {
        s.push("<");
        return i + 1;
    };

    if OPACAS.contains(&tag.nome) {
        if tag.fechamento {
            return tag.fim;
        }
        let fechando = format!("</{}", tag.nome);
        return match baixo[tag.fim..].find(&fechando) {
            Some(p) => {
                let depois = tag.fim + p;
                baixo[depois..]
                    .find('>')
                    .map(|q| depois + q + 1)
                    .unwrap_or(entrada.len())
            }
            None => tag.fim,
        };
    }

    match tag.nome {
        "br" => {
            s.quebra();
            tag.fim
        }
        "hr" => {
            s.bloco();
            if !s.em_linha() {
                s.push("---");
                s.quebra_dupla();
            }
            tag.fim
        }
        "img" => {
            if !tag.fechamento
                && let Some(src) = atributo(tag.attrs, "src")
            {
                let alt = atributo(tag.attrs, "alt").unwrap_or_default();
                s.push(&format!("![{}]({})", escapa_colchetes(&alt), src.trim()));
            }
            tag.fim
        }
        "pre" => {
            if tag.fechamento {
                return tag.fim;
            }
            // O conteúdo do `<pre>` é código: sai como bloco cercado, com as
            // tags internas (`<code>`, `<span>` de destaque) removidas.
            let (corpo, fim) = match baixo[tag.fim..].find("</pre") {
                Some(p) => {
                    let fecha = tag.fim + p;
                    let depois = baixo[fecha..]
                        .find('>')
                        .map(|q| fecha + q + 1)
                        .unwrap_or(entrada.len());
                    (&entrada[tag.fim..fecha], depois)
                }
                None => (&entrada[tag.fim..], entrada.len()),
            };
            let codigo = decodifica(&sem_tags(corpo));
            s.quebra_dupla();
            s.push("```\n");
            s.push(codigo.trim_matches('\n'));
            s.push("\n```");
            s.quebra_dupla();
            fim
        }
        "a" => {
            if tag.fechamento {
                if let Some((pos, href)) = s.links.pop() {
                    let vazio = s.txt[pos..].trim().len() <= usize::from(href.is_some());
                    match (href, vazio) {
                        // `<a>` só com espaço dentro: nem colchete nem link.
                        (Some(_), true) => s.txt.truncate(pos),
                        (Some(url), false) => {
                            // O espaço encostado na borda sai de dentro dos
                            // colchetes: `<a> <img> </a>` é um link para a
                            // imagem, não um link cujo texto começa com
                            // espaço.
                            let conteudo = s.txt[pos + 1..].to_string();
                            let antes = conteudo.starts_with(char::is_whitespace);
                            let depois = conteudo.ends_with(char::is_whitespace);
                            s.txt.truncate(pos);
                            if antes {
                                s.espaco();
                            }
                            s.push(&format!("[{}]({})", conteudo.trim(), url.trim()));
                            if depois {
                                s.txt.push(' ');
                            }
                        }
                        (None, _) => {}
                    }
                }
            } else {
                let href = atributo(tag.attrs, "href").filter(|h| !h.trim().is_empty());
                s.links.push((s.txt.len(), href.clone()));
                if href.is_some() {
                    s.push("[");
                }
            }
            tag.fim
        }
        "table" => {
            s.bloco();
            s.tabela = if tag.fechamento {
                None
            } else {
                Some(Tabela {
                    linhas: 0,
                    colunas: 0,
                })
            };
            if tag.fechamento {
                s.quebra_dupla();
            }
            tag.fim
        }
        "tr" => {
            if let Some(t) = &mut s.tabela {
                if tag.fechamento {
                    let colunas = t.colunas;
                    let primeira = t.linhas == 0;
                    t.linhas += 1;
                    s.push(" |\n");
                    if primeira && colunas > 0 {
                        s.push("|");
                        s.push(&" --- |".repeat(colunas));
                        s.push("\n");
                    }
                } else {
                    t.colunas = 0;
                }
            }
            tag.fim
        }
        "td" | "th" => {
            if let Some(t) = &mut s.tabela
                && !tag.fechamento
            {
                t.colunas += 1;
                let primeira = t.colunas == 1;
                s.push(if primeira { "| " } else { " | " });
            }
            tag.fim
        }
        "li" => {
            if tag.fechamento {
                s.quebra();
            } else {
                s.quebra();
                s.push("- ");
            }
            tag.fim
        }
        "dt" | "dd" => {
            s.quebra();
            tag.fim
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            s.bloco();
            if !tag.fechamento && !s.em_linha() {
                let nivel = tag.nome[1..].parse::<usize>().unwrap_or(3);
                s.push(&"#".repeat(nivel));
                s.push(" ");
            }
            tag.fim
        }
        "strong" | "b" => {
            s.push("**");
            tag.fim
        }
        "em" | "i" => {
            s.push("*");
            tag.fim
        }
        "del" | "s" => {
            s.push("~~");
            tag.fim
        }
        "code" => {
            s.push("`");
            tag.fim
        }
        "blockquote" | "p" | "div" | "section" | "article" | "header" | "footer" | "main"
        | "nav" | "aside" | "figure" | "figcaption" | "details" | "summary" | "center" | "ul"
        | "ol" | "dl" | "thead" | "tbody" | "tfoot" => {
            s.bloco();
            tag.fim
        }
        // Puramente decorativas (`<span>`, `<font>`, `<sup>`…): somem sem
        // deixar marca.
        _ => tag.fim,
    }
}

struct Tag<'a> {
    fechamento: bool,
    nome: &'a str,
    attrs: &'a str,
    fim: usize,
}

/// Lê `<tag ...>` a partir de `i`. `None` quando ali não há uma tag da lista
/// — o que mantém `<https://…>` e `<think>` como texto.
fn le_tag<'a>(entrada: &'a str, baixo: &'a str, i: usize) -> Option<Tag<'a>> {
    let b = baixo.as_bytes();
    let mut j = i + 1;
    let fechamento = b.get(j) == Some(&b'/');
    if fechamento {
        j += 1;
    }
    let ini_nome = j;
    while j < b.len() && b[j].is_ascii_alphanumeric() {
        j += 1;
    }
    let nome = baixo.get(ini_nome..j)?;
    if !TAGS.contains(&nome) {
        return None;
    }
    // Depois do nome só pode vir espaço, `/` ou `>`; `<pretenso>` não é
    // `<pre>`.
    match b.get(j) {
        Some(c) if c.is_ascii_whitespace() || *c == b'/' || *c == b'>' => {}
        _ => return None,
    }
    let ini_attrs = j;
    let mut aspas: Option<u8> = None;
    while j < b.len() {
        let c = b[j];
        match aspas {
            Some(q) if c == q => aspas = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => aspas = Some(c),
            None if c == b'>' => {
                return Some(Tag {
                    fechamento,
                    nome,
                    attrs: &entrada[ini_attrs..j],
                    fim: j + 1,
                });
            }
            None => {}
        }
        j += 1;
    }
    None // tag que nunca fechou: texto
}

/// O valor de um atributo, já com as entidades resolvidas.
fn atributo(attrs: &str, nome: &str) -> Option<String> {
    let baixo = attrs.to_ascii_lowercase();
    let b = baixo.as_bytes();
    let mut de = 0;
    while let Some(p) = baixo[de..].find(nome) {
        let p = de + p;
        de = p + nome.len();
        // `data-src` não é `src`.
        if p > 0 && !b[p - 1].is_ascii_whitespace() {
            continue;
        }
        let mut j = de;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) != Some(&b'=') {
            continue;
        }
        j += 1;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let valor = match b.get(j) {
            Some(&q) if q == b'"' || q == b'\'' => {
                let ini = j + 1;
                let fim = baixo[ini..].find(q as char).map(|k| ini + k)?;
                &attrs[ini..fim]
            }
            _ => {
                let ini = j;
                let fim = attrs[ini..]
                    .find(char::is_whitespace)
                    .map(|k| ini + k)
                    .unwrap_or(attrs.len());
                &attrs[ini..fim]
            }
        };
        return Some(decodifica(valor));
    }
    None
}

/// Remove tudo que pareça marcação — usado no miolo do `<pre>`, onde só o
/// texto interessa.
fn sem_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dentro = false;
    for c in s.chars() {
        match c {
            '<' => dentro = true,
            '>' => dentro = false,
            _ if !dentro => out.push(c),
            _ => {}
        }
    }
    out
}

/// `[` e `]` dentro do texto alternativo quebrariam o `![alt](src)`.
fn escapa_colchetes(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

/// Resolve a entidade que começa em `i`; devolve o texto e por onde seguir.
/// O que não é entidade conhecida volta como o próprio `&`.
fn entidade(entrada: &str, i: usize) -> (String, usize) {
    let limite = (i + 12).min(entrada.len());
    let Some(fim) = entrada[i..limite].find(';').map(|p| i + p) else {
        return ("&".to_string(), i + 1);
    };
    let nome = &entrada[i + 1..fim];
    let texto = match nome.to_ascii_lowercase().as_str() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "#39" => "'",
        "nbsp" | "#160" => " ",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "times" => "×",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "bull" => "•",
        "middot" => "·",
        "larr" => "←",
        "rarr" => "→",
        "laquo" => "«",
        "raquo" => "»",
        "ldquo" => "“",
        "rdquo" => "”",
        "lsquo" => "‘",
        "rsquo" => "’",
        outro => {
            // Numérica: `&#8212;` ou `&#x2014;`.
            let cp = outro
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse::<u32>().ok(),
                });
            return match cp.and_then(char::from_u32) {
                Some(c) => (c.to_string(), fim + 1),
                None => ("&".to_string(), i + 1),
            };
        }
    };
    (texto.to_string(), fim + 1)
}

fn decodifica(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i..].find('&').map(|p| i + p) {
            Some(p) => {
                out.push_str(&s[i..p]);
                let (texto, fim) = entidade(s, p);
                out.push_str(&texto);
                i = fim;
            }
            None => {
                out.push_str(&s[i..]);
                break;
            }
        }
    }
    out
}

/// Aperta o resultado: qualquer vão com duas ou mais quebras vira uma linha
/// em branco só. Vãos de uma quebra ficam como estão — em Markdown, os dois
/// espaços no fim da linha são uma quebra deliberada do autor.
fn normaliza(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            let ini = i;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let vao = &s[ini..i];
            if vao.matches('\n').count() >= 2 {
                out.push_str("\n\n");
                // A indentação da linha seguinte sobrevive: quatro espaços
                // depois de uma linha em branco são um bloco de código, e a
                // continuação de um item de lista depende dela.
                if let Some(p) = vao.rfind('\n') {
                    out.push_str(&vao[p + 1..]);
                }
            } else {
                out.push_str(vao);
            }
        } else {
            let ini = i;
            while i < b.len() && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push_str(&s[ini..i]);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um recorte REAL do cartão do unsloth — o mesmo que aparecia na tela
    /// como `<div> <p style="margin: 0 0 0px 0">`.
    #[test]
    fn a_real_model_card_stops_arriving_as_raw_html() {
        let card = r#"Read our How to <a href="https://docs.unsloth.ai/guide">Run Qwen3.8-27B Guide!</a>

<div> <p style="margin: 0 0 0px 0; margin-top: 0px;"> <em><a href="https://unsloth.ai/docs/basics/dynamic-3.0-ggufs">Unsloth Dynamic 3.0</a> achieves superior accuracy</em> </p> <div style="display: flex; gap: 5px; align-items: center;"> <a href="https://github.com/unslothai/unsloth/"> <img src="https://github.com/unslothai/unsloth/raw/main/images/unsloth%20new%20logo.png" width="133"> </a> </div> <ul style="margin: 0;"> <li>Introducing <a href="https://unsloth.ai/docs">Dynamic V3.0</a> GGUFs</li> <li>Tool calling improvements &amp; nested objects</li> </ul> </div>"#;

        let md = card_em_markdown(card);

        // Nenhuma marcação sobra para a tela interpretar.
        assert!(!md.contains('<'), "sobrou HTML: {md}");
        assert!(!md.contains("style="), "sobrou atributo: {md}");
        // O link do topo, que já era Markdown-com-HTML, virou link.
        assert!(md.contains("[Run Qwen3.8-27B Guide!](https://docs.unsloth.ai/guide)"));
        // Ênfase, lista e entidade chegaram legíveis.
        assert!(md.contains("*[Unsloth Dynamic 3.0](https://unsloth.ai/docs/basics/dynamic-3.0-ggufs) achieves superior accuracy*"));
        assert!(md.contains("- Introducing [Dynamic V3.0](https://unsloth.ai/docs) GGUFs"));
        assert!(md.contains("Tool calling improvements & nested objects"));
        // A imagem dentro do link virou link-de-imagem, não colchete vazio.
        assert!(md.contains(
            "[![](https://github.com/unslothai/unsloth/raw/main/images/unsloth%20new%20logo.png)](https://github.com/unslothai/unsloth/)"
        ));
    }

    /// Markdown que já era Markdown atravessa sem uma vírgula fora do lugar.
    #[test]
    fn plain_markdown_survives_untouched() {
        let md = "# Título\n\nUm parágrafo com `código <div>` e um [link](https://a.b).\n\n- item\n- outro\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```bash\nllama-server -m modelo.gguf --host <ip>\n```\n\nFim.";
        assert_eq!(card_em_markdown(md), md);
    }

    /// Dentro de um bloco cercado, `<div>` é código do autor — e código não
    /// se reescreve.
    #[test]
    fn fenced_blocks_are_never_rewritten() {
        let md = "```html\n<div class=\"x\">&amp;</div>\n```";
        assert_eq!(card_em_markdown(md), md);
    }

    /// O que não está na lista de tags é texto: um autolink e um `<think>`
    /// continuam visíveis.
    #[test]
    fn unknown_angle_brackets_stay_as_text() {
        let md = "Veja <https://unsloth.ai> e a marca <think> do template.";
        assert_eq!(card_em_markdown(md), md);
    }

    /// Tabela HTML vira tabela GFM — inclusive a linha separadora, que o
    /// HTML não tem e sem a qual o GFM mostra uma parede de pipes.
    #[test]
    fn html_tables_become_gfm_tables() {
        let card = "<table><tr><th>Quant</th><th>Tamanho</th></tr><tr><td>Q4_K_M</td><td>16 GB</td></tr></table>";
        assert_eq!(
            card_em_markdown(card),
            "| Quant | Tamanho |\n| --- | --- |\n| Q4_K_M | 16 GB |"
        );
    }

    /// Uma quebra dentro da célula não pode partir a linha da tabela.
    #[test]
    fn a_break_inside_a_cell_does_not_split_the_row() {
        let card = "<table><tr><td>a<br>b</td><td>c</td></tr></table>";
        assert!(card_em_markdown(card).lines().count() == 2);
    }

    /// `<pre>` guarda comando de terminal: sai como bloco cercado, com as
    /// entidades resolvidas.
    #[test]
    fn pre_becomes_a_fenced_block() {
        let card = "<pre><code>llama-cli -m a.gguf --ctx 4096 &amp;&amp; echo ok</code></pre>";
        assert_eq!(
            card_em_markdown(card),
            "```\nllama-cli -m a.gguf --ctx 4096 && echo ok\n```"
        );
    }

    /// `<script>` e `<style>` somem com o conteúdo: nada ali é descrição.
    #[test]
    fn script_and_style_leave_nothing_behind() {
        let card = "Antes<script>alert('x')</script><style>.a{color:red}</style>Depois";
        assert_eq!(card_em_markdown(card), "AntesDepois");
    }

    /// Um `<a>` com href vazio ou ausente vira texto, não `[texto]()`.
    #[test]
    fn a_link_without_a_target_is_just_text() {
        assert_eq!(card_em_markdown("<a>texto</a>"), "texto");
        assert_eq!(card_em_markdown("<a href=''>texto</a>"), "texto");
    }

    /// Cabeçalho HTML vira cabeçalho Markdown no nível certo.
    #[test]
    fn html_headings_keep_their_level() {
        assert_eq!(
            card_em_markdown("<h2>Uso</h2><p>Texto</p>"),
            "## Uso\n\nTexto"
        );
    }

    /// A indentação com que o autor formatou o HTML não pode virar bloco de
    /// código: quatro espaços depois de uma linha em branco, em Markdown, é
    /// exatamente isso — e o cartão inteiro apareceria monoespaçado.
    #[test]
    fn html_indentation_does_not_become_a_code_block() {
        let card = "<div>\n    <p>\n        Texto indentado no fonte.\n    </p>\n</div>";
        assert_eq!(card_em_markdown(card), "Texto indentado no fonte.");
    }

    /// Mas a indentação do Markdown, essa é do autor e fica.
    #[test]
    fn markdown_indentation_is_preserved() {
        let md = "Exemplo:\n\n    llama-server -m a.gguf\n\nFim.";
        assert_eq!(card_em_markdown(md), md);
    }

    /// Comentário de HTML não é descrição de modelo.
    #[test]
    fn comments_are_dropped() {
        assert_eq!(card_em_markdown("a<!-- oculto -->b"), "ab");
    }

    /// Entidades numéricas e nomeadas viram o caractere que representam.
    #[test]
    fn entities_become_characters() {
        assert_eq!(
            card_em_markdown("5 &lt; 10 &amp;&amp; &#8212; &#x2192; caf&eacute;?"),
            "5 < 10 && — → caf&eacute;?"
        );
    }
}
