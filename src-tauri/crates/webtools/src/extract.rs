//! HTML → texto legível, sem dependência de parser.
//!
//! O modelo não precisa do documento: precisa do que está escrito nele. HTML
//! cru gasta o contexto inteiro com `<div class="…">` e ainda esconde o texto
//! no meio de `<script>` — e script é justamente onde mora o lixo (JSON de
//! telemetria, código minificado) que faz um modelo pequeno se perder.
//!
//! Por que um varredor próprio e não uma crate de parsing: para *extrair
//! texto* basta saber onde uma tag começa e termina, o que dá ~150 linhas.
//! Um parser de verdade (árvore, correção de erros, seletores) traria dezenas
//! de dependências transitivas para um ganho que não muda o resultado — o
//! modelo lê o mesmo parágrafo nos dois casos.
//!
//! O que a extração faz, em ordem:
//! 1. Descarta o conteúdo de elementos que nunca são leitura ([`SKIPPED`]):
//!    `script`, `style`, `nav`, `header`, `footer`, `aside`, formulários.
//! 2. Se existir `<main>` ou `<article>`, lê **só** essa região — é a
//!    heurística que separa o artigo do resto do site.
//! 3. Transforma estrutura em pontuação: parágrafo vira linha em branco,
//!    `<li>` vira `- `, `<h2>` vira `## `.
//!
//! Limite conhecido e aceito: página que renderiza tudo por JavaScript volta
//! quase vazia. O resultado diz isso em vez de fingir sucesso.

/// Elementos cujo conteúdo inteiro é descartado.
const SKIPPED: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "math", "iframe", "object", "embed",
    "canvas", "video", "audio", "head", "nav", "header", "footer", "aside", "form", "button",
    "select", "option", "datalist", "dialog", "menu",
];

/// Elementos cujo texto interno é literal (não tem tag dentro).
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

/// Elementos que quebram linha ao abrir e ao fechar.
const BLOCK: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "table",
    "thead",
    "tbody",
    "tr",
    "blockquote",
    "pre",
    "figure",
    "figcaption",
    "address",
    "hr",
    "details",
    "summary",
    "fieldset",
    "label",
];

/// Um pedaço do documento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    Close {
        name: String,
    },
    Text(String),
}

impl Node {
    /// Valor de um atributo (nome em minúsculas).
    pub fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Node::Open { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    /// A tag abre com uma dessas classes?
    pub fn has_class(&self, wanted: &[&str]) -> bool {
        let Some(class) = self.attr("class") else {
            return false;
        };
        class.split_whitespace().any(|c| wanted.contains(&c))
    }
}

/// Quebra o HTML em tags e texto.
///
/// Trabalha em bytes ASCII (`<`, `>`, aspas): como UTF-8 nunca usa esses
/// bytes dentro de um caractere multibyte, fatiar nesses pontos é seguro.
pub fn tokenize(html: &str) -> Vec<Node> {
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < len {
        if bytes[i] != b'<' {
            let start = i;
            while i < len && bytes[i] != b'<' {
                i += 1;
            }
            let text = &html[start..i];
            if !text.trim().is_empty() {
                out.push(Node::Text(decode_entities(text)));
            } else if !text.is_empty() {
                out.push(Node::Text(" ".to_string()));
            }
            continue;
        }

        // Comentário, doctype, CDATA: fora.
        if html[i..].starts_with("<!--") {
            i = match html[i + 4..].find("-->") {
                Some(pos) => i + 4 + pos + 3,
                None => len,
            };
            continue;
        }
        if i + 1 < len && (bytes[i + 1] == b'!' || bytes[i + 1] == b'?') {
            i = match html[i..].find('>') {
                Some(pos) => i + pos + 1,
                None => len,
            };
            continue;
        }

        // Tag de fechamento.
        if i + 1 < len && bytes[i + 1] == b'/' {
            let mut j = i + 2;
            while j < len && is_name_byte(bytes[j]) {
                j += 1;
            }
            let name = html[i + 2..j].to_ascii_lowercase();
            i = match html[j..].find('>') {
                Some(pos) => j + pos + 1,
                None => len,
            };
            if !name.is_empty() {
                out.push(Node::Close { name });
            }
            continue;
        }

        // Tag de abertura (ou um `<` solto, que vira texto).
        if i + 1 < len && bytes[i + 1].is_ascii_alphabetic() {
            let (node, next) = parse_open_tag(html, i);
            i = next;
            let raw = match &node {
                Node::Open {
                    name,
                    self_closing: false,
                    ..
                } if RAW_TEXT.contains(&name.as_str()) => Some(name.clone()),
                _ => None,
            };
            out.push(node);
            if let Some(name) = raw {
                let end = find_close(html, i, &name).unwrap_or(len);
                let text = &html[i..end];
                if !text.trim().is_empty() {
                    out.push(Node::Text(decode_entities(text)));
                }
                i = end;
            }
            continue;
        }

        out.push(Node::Text("<".to_string()));
        i += 1;
    }

    out
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

/// Lê `<nome attr="valor" …>` a partir de `start`, devolvendo o nó e a
/// posição logo depois de `>`.
fn parse_open_tag(html: &str, start: usize) -> (Node, usize) {
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut i = start + 1;
    let name_start = i;
    while i < len && is_name_byte(bytes[i]) {
        i += 1;
    }
    let name = html[name_start..i].to_ascii_lowercase();

    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;

    loop {
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        if bytes[i] == b'/' {
            self_closing = true;
            i += 1;
            continue;
        }

        // A barra também encerra o nome: em `<img alt=x hidden/>` o `/` é o
        // fecha-tag, não parte do atributo.
        let key_start = i;
        while i < len
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == key_start {
            // Caractere inesperado: pula para não travar.
            i += 1;
            continue;
        }
        let key = html[key_start..i].to_ascii_lowercase();

        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < len && bytes[i] == b'=' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < len && bytes[i] != quote {
                    i += 1;
                }
                value = decode_entities(&html[v_start..i]);
                if i < len {
                    i += 1;
                }
            } else {
                let v_start = i;
                while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = decode_entities(&html[v_start..i]);
            }
        }
        attrs.push((key, value));
    }

    (
        Node::Open {
            name,
            attrs,
            self_closing,
        },
        i,
    )
}

/// Acha `</nome` a partir de `from`, ignorando maiúsculas.
fn find_close(html: &str, from: usize, name: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' {
            let rest = &html[i + 2..];
            if rest.len() >= name.len() && rest[..name.len()].eq_ignore_ascii_case(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Troca entidades HTML pelo caractere correspondente.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        match after.find(';') {
            // Entidade longa demais é `&` literal seguido de texto.
            Some(end) if end <= 10 => {
                let name = &after[..end];
                match named_entity(name) {
                    Some(ch) => out.push_str(ch),
                    None => match numeric_entity(name) {
                        Some(ch) => out.push(ch),
                        None => {
                            out.push('&');
                            out.push_str(name);
                            out.push(';');
                        }
                    },
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "lsquo" | "rsquo" => "'",
        "ldquo" | "rdquo" => "\"",
        "laquo" => "«",
        "raquo" => "»",
        "middot" => "·",
        "bull" => "•",
        "times" => "×",
        "deg" => "°",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "euro" => "€",
        "pound" => "£",
        "aacute" => "á",
        "eacute" => "é",
        "iacute" => "í",
        "oacute" => "ó",
        "uacute" => "ú",
        "atilde" => "ã",
        "otilde" => "õ",
        "ccedil" => "ç",
        "ecirc" => "ê",
        "ocirc" => "ô",
        "acirc" => "â",
        "agrave" => "à",
        _ => return None,
    })
}

fn numeric_entity(name: &str) -> Option<char> {
    let digits = name.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

/// Resultado da extração.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readable {
    /// Conteúdo de `<title>`, quando existe.
    pub title: Option<String>,
    /// Texto principal, já com quebras de linha úteis.
    pub text: String,
}

/// Extrai o texto legível de um documento HTML.
pub fn html_to_text(html: &str) -> Readable {
    let nodes = tokenize(html);
    let title = extract_title(&nodes);
    let (start, end) = main_region(&nodes);
    let text = render(&nodes[start..end]);
    Readable { title, text }
}

fn extract_title(nodes: &[Node]) -> Option<String> {
    let mut iter = nodes.iter().peekable();
    while let Some(node) = iter.next() {
        if matches!(node, Node::Open { name, .. } if name == "title")
            && let Some(Node::Text(t)) = iter.peek()
        {
            let title = collapse_ws(t);
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

/// Região que vale a pena ler: `<main>`, senão `<article>`, senão `<body>`.
fn main_region(nodes: &[Node]) -> (usize, usize) {
    for tag in ["main", "article", "body"] {
        if let Some(range) = region_of(nodes, tag) {
            // Região vazia (ex.: `<main>` só com script) não serve.
            if range.1 > range.0 + 1 {
                return range;
            }
        }
    }
    (0, nodes.len())
}

fn region_of(nodes: &[Node], tag: &str) -> Option<(usize, usize)> {
    let start = nodes.iter().position(|n| match n {
        Node::Open {
            name,
            self_closing: false,
            ..
        } => name == tag,
        _ => false,
    })?;
    let mut depth = 0usize;
    for (i, node) in nodes.iter().enumerate().skip(start) {
        match node {
            Node::Open {
                name,
                self_closing: false,
                ..
            } if name == tag => depth += 1,
            Node::Close { name } if name == tag => {
                depth -= 1;
                if depth == 0 {
                    return Some((start + 1, i));
                }
            }
            _ => {}
        }
    }
    Some((start + 1, nodes.len()))
}

fn render(nodes: &[Node]) -> String {
    let mut out = String::new();
    // Nome do elemento sendo pulado + profundidade dele. Contador em vez de
    // pilha completa: HTML malformado desbalanceia pilha, mas um contador do
    // mesmo nome se recupera sozinho.
    let mut skip: Option<(String, usize)> = None;
    let mut pre_depth = 0usize;

    for node in nodes {
        if let Some((tag, depth)) = &mut skip {
            match node {
                Node::Open {
                    name,
                    self_closing: false,
                    ..
                } if name == tag => *depth += 1,
                Node::Close { name } if name == tag => {
                    *depth -= 1;
                    if *depth == 0 {
                        skip = None;
                    }
                }
                _ => {}
            }
            continue;
        }

        match node {
            Node::Open {
                name, self_closing, ..
            } => {
                if SKIPPED.contains(&name.as_str()) {
                    if !self_closing {
                        skip = Some((name.clone(), 1));
                    }
                    continue;
                }
                match name.as_str() {
                    "br" => out.push('\n'),
                    "li" => {
                        newline(&mut out);
                        out.push_str("- ");
                    }
                    "pre" => {
                        pre_depth += 1;
                        blank_line(&mut out);
                    }
                    "td" | "th" => {
                        if !out.ends_with('\n') && !out.is_empty() {
                            out.push_str(" | ");
                        }
                    }
                    h if is_heading(h) => {
                        blank_line(&mut out);
                        let level = h[1..].parse::<usize>().unwrap_or(1);
                        out.push_str(&"#".repeat(level.clamp(1, 6)));
                        out.push(' ');
                    }
                    b if BLOCK.contains(&b) => blank_line(&mut out),
                    _ => {}
                }
            }
            Node::Close { name } => match name.as_str() {
                "pre" => {
                    pre_depth = pre_depth.saturating_sub(1);
                    blank_line(&mut out);
                }
                "li" => newline(&mut out),
                h if is_heading(h) => blank_line(&mut out),
                b if BLOCK.contains(&b) => blank_line(&mut out),
                _ => {}
            },
            Node::Text(text) => {
                if pre_depth > 0 {
                    out.push_str(text);
                } else {
                    let piece = collapse_ws(text);
                    if piece.is_empty() {
                        continue;
                    }
                    let needs_space = !out.is_empty()
                        && !out.ends_with(['\n', ' '])
                        && !piece.starts_with(|c: char| ".,;:!?)".contains(c));
                    if needs_space {
                        out.push(' ');
                    }
                    out.push_str(&piece);
                }
            }
        }
    }

    tidy(&out)
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn newline(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn blank_line(out: &mut String) {
    if out.is_empty() {
        return;
    }
    newline(out);
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
}

/// Junta espaços/quebras de um nó de texto num espaço só.
pub fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    if space && !out.is_empty() {
        out.push(' ');
    }
    out.trim().to_string()
}

/// Limpa o texto final: espaços no fim da linha, `|` sobrando e mais de uma
/// linha em branco seguida.
fn tidy(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let cleaned = line.trim_end().trim_end_matches('|').trim_end().to_string();
        let blank = cleaned.trim().is_empty();
        if blank && lines.last().map(|l: &String| l.trim().is_empty()) == Some(true) {
            continue;
        }
        lines.push(if blank { String::new() } else { cleaned });
    }
    while lines.first().map(|l| l.trim().is_empty()) == Some(true) {
        lines.remove(0);
    }
    while lines.last().map(|l| l.trim().is_empty()) == Some(true) {
        lines.pop();
    }
    lines.join("\n")
}

/// Corta o texto em `max_chars` caracteres (não bytes), avisando o corte.
pub fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_string(), false);
    }
    let cut: String = text.chars().take(max_chars).collect();
    (cut, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
<!DOCTYPE html>
<html lang="pt-BR">
<head>
  <title>Guia do Agente</title>
  <style>body { color: red; }</style>
  <script>var rastreio = {"id": 7}; if (a < b) { alerta("nao apareca"); }</script>
</head>
<body>
  <nav><a href="/inicio">Início</a> <a href="/precos">Preços</a></nav>
  <header><span>menu do topo</span></header>
  <main>
    <h1>Como usar</h1>
    <p>Primeiro par&aacute;grafo com &amp; e &#233;.</p>
    <ul><li>Passo um</li><li>Passo dois</li></ul>
    <pre><code>cargo test
cargo clippy</code></pre>
    <p>Fim do texto.</p>
  </main>
  <footer>&copy; 2026 Rodap&eacute;</footer>
  <script>outro_rastreio();</script>
</body>
</html>
"#;

    #[test]
    fn drops_script_style_nav_header_and_footer() {
        let out = html_to_text(PAGE);
        assert!(!out.text.contains("rastreio"), "{}", out.text);
        assert!(!out.text.contains("color: red"), "{}", out.text);
        assert!(!out.text.contains("nao apareca"), "{}", out.text);
        assert!(!out.text.contains("Preços"), "nav ficou: {}", out.text);
        assert!(!out.text.contains("menu do topo"), "{}", out.text);
        assert!(!out.text.contains("Rodapé"), "{}", out.text);
    }

    #[test]
    fn keeps_the_readable_content_with_structure() {
        let out = html_to_text(PAGE);
        assert_eq!(out.title.as_deref(), Some("Guia do Agente"));
        assert!(out.text.contains("# Como usar"), "{}", out.text);
        assert!(out.text.contains("- Passo um"), "{}", out.text);
        assert!(out.text.contains("- Passo dois"), "{}", out.text);
        assert!(out.text.contains("cargo clippy"), "pre: {}", out.text);
        assert!(out.text.contains("Fim do texto."), "{}", out.text);
        // Parágrafos separados por linha em branco, sem paredão de texto.
        assert!(out.text.contains("\n\n"), "{}", out.text);
    }

    #[test]
    fn decodes_entities_in_text_and_attributes() {
        let out = html_to_text(PAGE);
        assert!(out.text.contains("parágrafo"), "{}", out.text);
        assert!(out.text.contains("com & e é"), "{}", out.text);

        assert_eq!(
            decode_entities("a &lt; b &amp;&amp; c &gt; d"),
            "a < b && c > d"
        );
        assert_eq!(decode_entities("&#x41;&#66;"), "AB");
        // `&` solto e entidade desconhecida ficam como estão.
        assert_eq!(decode_entities("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(decode_entities("&naoexiste;"), "&naoexiste;");
    }

    #[test]
    fn prefers_the_main_region_over_the_whole_page() {
        let html = "<body><div>ruído de barra lateral</div>\
                    <article><p>o artigo mesmo</p></article>\
                    <div>rodapé solto</div></body>";
        let out = html_to_text(html);
        assert!(out.text.contains("o artigo mesmo"), "{}", out.text);
        assert!(!out.text.contains("ruído"), "{}", out.text);
        assert!(!out.text.contains("rodapé solto"), "{}", out.text);
    }

    #[test]
    fn tokenizer_survives_comments_doctype_and_loose_attributes() {
        let nodes = tokenize(
            "<!-- comentário <b>com tag</b> --><!DOCTYPE html>\
             <img src=foto.png alt='uma foto' hidden/><p class=\"a b\">oi</p>",
        );
        assert!(
            !nodes
                .iter()
                .any(|n| matches!(n, Node::Text(t) if t.contains("comentário"))),
            "comentário virou texto: {nodes:?}"
        );
        let img = nodes
            .iter()
            .find(|n| matches!(n, Node::Open { name, .. } if name == "img"))
            .unwrap();
        assert_eq!(img.attr("src"), Some("foto.png"));
        assert_eq!(img.attr("alt"), Some("uma foto"));
        assert_eq!(img.attr("hidden"), Some(""));
        assert!(matches!(
            img,
            Node::Open {
                self_closing: true,
                ..
            }
        ));

        let p = nodes
            .iter()
            .find(|n| matches!(n, Node::Open { name, .. } if name == "p"))
            .unwrap();
        assert!(p.has_class(&["b"]), "classe múltipla: {p:?}");
        assert!(!p.has_class(&["c"]));
    }

    #[test]
    fn plain_text_documents_pass_through() {
        let out = html_to_text("apenas texto, sem nenhuma tag");
        assert_eq!(out.text, "apenas texto, sem nenhuma tag");
        assert!(out.title.is_none());
    }

    #[test]
    fn javascript_only_pages_end_up_empty_instead_of_lying() {
        let out =
            html_to_text("<html><body><div id=\"root\"></div><script>app()</script></body></html>");
        assert!(out.text.trim().is_empty(), "{}", out.text);
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        let (cut, truncated) = truncate_chars("çãéíõ", 3);
        assert!(truncated);
        assert_eq!(cut, "çãé");
        let (whole, truncated) = truncate_chars("abc", 10);
        assert!(!truncated);
        assert_eq!(whole, "abc");
    }
}
