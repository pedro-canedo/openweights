//! Leitura dos counters do `GET /metrics` (Prometheus) do llama-server e o
//! acumulador de deltas que os transforma em totais que nunca regridem.
//!
//! Fatos do b10441 que moldam este módulo:
//!
//! - Os gauges `llamacpp:prompt_tokens_seconds`/`predicted_tokens_seconds`
//!   são média DA JANELA entre scrapes, e o próprio scrape ZERA o bucket
//!   (server-context.cpp:4453-4455). Por isso aqui entram SÓ os counters
//!   cumulativos, e as médias saem por divisão (tokens ÷ segundos) — imune ao
//!   efeito de janela e a scrapes de Prometheus de terceiros.
//! - A direção reversa não é neutra: cada scrape NOSSO zera o bucket desses
//!   dois gauges para qualquer outro consumidor. Quem monitora o mesmo
//!   servidor com Prometheus próprio deve usar `rate()` sobre os `*_total`.
//! - Counters vivem no processo e zeram quando ele reinicia — o
//!   [`DeltaTracker`] detecta a queda e trata a leitura nova como delta
//!   inteiro, então os acumuladores de quem o usa nunca regridem.
//! - O valor é serializado como double com precisão default de 6 dígitos:
//!   a partir de 1e6 ele chega em notação científica (`1.23457e+06`), e o
//!   parse é `f64` de propósito.

use std::collections::HashMap;

/// Os 5 counters cumulativos do `/metrics` que interessam ao app.
///
/// `prompt_tokens` EXCLUI os reaproveitados do cache (mesmo recorte do
/// upstream); `cached_tokens` são exatamente esses reaproveitados.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MetricCounters {
    /// `llamacpp:prompt_tokens_total`
    pub prompt_tokens: f64,
    /// `llamacpp:prompt_tokens_cached_total`
    pub cached_tokens: f64,
    /// `llamacpp:prompt_seconds_total`
    pub prompt_seconds: f64,
    /// `llamacpp:tokens_predicted_total`
    pub predicted_tokens: f64,
    /// `llamacpp:tokens_predicted_seconds_total`
    pub predicted_seconds: f64,
}

impl MetricCounters {
    /// Soma campo a campo — é como um delta entra num acumulador.
    pub fn add(&mut self, outro: &MetricCounters) {
        self.prompt_tokens += outro.prompt_tokens;
        self.cached_tokens += outro.cached_tokens;
        self.prompt_seconds += outro.prompt_seconds;
        self.predicted_tokens += outro.predicted_tokens;
        self.predicted_seconds += outro.predicted_seconds;
    }

    /// Delta todo zerado não merece escrita em banco.
    pub fn is_zero(&self) -> bool {
        self.prompt_tokens == 0.0
            && self.cached_tokens == 0.0
            && self.prompt_seconds == 0.0
            && self.predicted_tokens == 0.0
            && self.predicted_seconds == 0.0
    }
}

/// Extrai os 5 counters do corpo texto do `/metrics`.
///
/// O formato esperado é `llamacpp:<nome> <valor>` SEM labels — é como o
/// b10441 emite. Linha com label (`nome{...}`) é ignorada de propósito: se
/// uma versão futura rotular os counters, é melhor coletar zero (e revisar o
/// parser) do que somar séries diferentes como se fossem uma. Comentários
/// (`#`), counters desconhecidos e valores ilegíveis são pulados.
pub fn parse_metrics(body: &str) -> MetricCounters {
    let mut out = MetricCounters::default();
    for linha in body.lines() {
        let linha = linha.trim();
        if linha.is_empty() || linha.starts_with('#') {
            continue;
        }
        let mut partes = linha.split_whitespace();
        let (Some(nome), Some(valor)) = (partes.next(), partes.next()) else {
            continue;
        };
        // Label = série rotulada, que este parser não sabe agregar.
        if nome.contains('{') {
            continue;
        }
        // f64 aceita a notação científica que o upstream emite acima de 1e6.
        let Ok(v) = valor.parse::<f64>() else {
            continue;
        };
        match nome {
            "llamacpp:prompt_tokens_total" => out.prompt_tokens = v,
            "llamacpp:prompt_tokens_cached_total" => out.cached_tokens = v,
            "llamacpp:prompt_seconds_total" => out.prompt_seconds = v,
            "llamacpp:tokens_predicted_total" => out.predicted_tokens = v,
            "llamacpp:tokens_predicted_seconds_total" => out.predicted_seconds = v,
            _ => {}
        }
    }
    out
}

/// Última leitura por (modelo, campo), para converter counters cumulativos em
/// deltas — com detecção de reset por campo: valor que CAIU significa
/// processo novo (counters recomeçaram do zero), e o delta é a leitura
/// inteira.
///
/// Quem limpa acumuladores (o "Clear" da tela) NÃO pode limpar este tracker:
/// sem a última leitura, o próximo scrape devolveria o counter inteiro como
/// delta e recontaria tráfego que já foi somado antes da limpeza.
#[derive(Debug, Default)]
pub struct DeltaTracker {
    ultimo: HashMap<String, MetricCounters>,
}

impl DeltaTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra a leitura de agora e devolve o delta sobre a anterior.
    ///
    /// Primeira leitura de um modelo: delta = leitura inteira (os counters
    /// nasceram zerados junto com o processo, que este app mesmo subiu).
    pub fn delta(&mut self, model: &str, atual: MetricCounters) -> MetricCounters {
        let anterior = self
            .ultimo
            .insert(model.to_string(), atual)
            .unwrap_or_default();
        MetricCounters {
            prompt_tokens: campo_delta(anterior.prompt_tokens, atual.prompt_tokens),
            cached_tokens: campo_delta(anterior.cached_tokens, atual.cached_tokens),
            prompt_seconds: campo_delta(anterior.prompt_seconds, atual.prompt_seconds),
            predicted_tokens: campo_delta(anterior.predicted_tokens, atual.predicted_tokens),
            predicted_seconds: campo_delta(anterior.predicted_seconds, atual.predicted_seconds),
        }
    }
}

/// `atual >= ultimo` → progresso normal; queda → o processo reiniciou e o
/// counter recomeçou, então tudo que ele mostra agora é tráfego novo.
fn campo_delta(ultimo: f64, atual: f64) -> f64 {
    if atual >= ultimo {
        atual - ultimo
    } else {
        atual
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Corpo no formato do b10441: comentários, os 5 counters (um deles em
    /// notação científica, como o upstream emite acima de 1e6), gauges de
    /// janela que NÃO podem entrar e uma linha rotulada a ignorar.
    #[test]
    fn the_five_counters_are_read_and_everything_else_is_ignored() {
        let corpo = "\
# HELP llamacpp:prompt_tokens_total Number of prompt tokens processed.
# TYPE llamacpp:prompt_tokens_total counter
llamacpp:prompt_tokens_total 1.23457e+06
llamacpp:prompt_tokens_cached_total 5000
llamacpp:prompt_seconds_total 42.5
llamacpp:tokens_predicted_total 987
llamacpp:tokens_predicted_seconds_total 12.25
llamacpp:prompt_tokens_seconds 512
llamacpp:predicted_tokens_seconds 33
llamacpp:requests_processing 0
llamacpp:prompt_tokens_total{model=\"outro\"} 999999
";
        let c = parse_metrics(corpo);
        // 1.23457e+06 = 1234570: o parse f64 engole a notação científica.
        assert!((c.prompt_tokens - 1_234_570.0).abs() < 1e-6);
        assert!((c.cached_tokens - 5000.0).abs() < f64::EPSILON);
        assert!((c.prompt_seconds - 42.5).abs() < f64::EPSILON);
        assert!((c.predicted_tokens - 987.0).abs() < f64::EPSILON);
        assert!((c.predicted_seconds - 12.25).abs() < f64::EPSILON);
    }

    /// A linha rotulada não pode nem sobrescrever nem valer sozinha: série
    /// com label é outra série, e este parser só conhece as lisas.
    #[test]
    fn a_labeled_line_never_becomes_the_value() {
        let so_rotulada = "llamacpp:prompt_tokens_total{model=\"x\"} 777\n";
        assert!(parse_metrics(so_rotulada).is_zero());

        let corpo = "llamacpp:prompt_tokens_total 10\n\
                     llamacpp:prompt_tokens_total{model=\"x\"} 777\n";
        assert!((parse_metrics(corpo).prompt_tokens - 10.0).abs() < f64::EPSILON);
    }

    /// Corpo vazio, lixo e valor ilegível: tudo vira zero, nunca pânico.
    #[test]
    fn garbage_bodies_parse_to_zero() {
        assert!(parse_metrics("").is_zero());
        assert!(parse_metrics("não é prometheus\n\n###").is_zero());
        assert!(parse_metrics("llamacpp:prompt_tokens_total abc").is_zero());
        assert!(parse_metrics("llamacpp:prompt_tokens_total").is_zero());
    }

    fn leitura(prompt: f64, predicted: f64) -> MetricCounters {
        MetricCounters {
            prompt_tokens: prompt,
            cached_tokens: prompt / 10.0,
            prompt_seconds: prompt / 100.0,
            predicted_tokens: predicted,
            predicted_seconds: predicted / 10.0,
        }
    }

    /// Primeira leitura entra inteira; a segunda vira diferença.
    #[test]
    fn deltas_are_differences_after_the_first_reading() {
        let mut t = DeltaTracker::new();
        let d1 = t.delta("m.gguf", leitura(100.0, 50.0));
        assert!((d1.prompt_tokens - 100.0).abs() < f64::EPSILON);
        assert!((d1.predicted_tokens - 50.0).abs() < f64::EPSILON);

        let d2 = t.delta("m.gguf", leitura(160.0, 80.0));
        assert!((d2.prompt_tokens - 60.0).abs() < f64::EPSILON);
        assert!((d2.predicted_tokens - 30.0).abs() < f64::EPSILON);
        assert!((d2.cached_tokens - 6.0).abs() < 1e-9);

        // Sem tráfego novo: delta zero, e é isso que evita escrita à toa.
        assert!(t.delta("m.gguf", leitura(160.0, 80.0)).is_zero());
    }

    /// O processo reiniciou: o counter caiu, e a leitura nova é toda delta.
    /// O acumulador de quem usa nunca regride.
    #[test]
    fn a_counter_drop_means_restart_and_the_new_value_is_the_delta() {
        let mut t = DeltaTracker::new();
        t.delta("m.gguf", leitura(500.0, 200.0));
        let d = t.delta("m.gguf", leitura(30.0, 12.0));
        assert!((d.prompt_tokens - 30.0).abs() < f64::EPSILON);
        assert!((d.predicted_tokens - 12.0).abs() < f64::EPSILON);
        // E a vida segue a partir da leitura nova.
        let d = t.delta("m.gguf", leitura(50.0, 20.0));
        assert!((d.prompt_tokens - 20.0).abs() < f64::EPSILON);
    }

    /// Modelos diferentes têm trilhas independentes.
    #[test]
    fn each_model_tracks_its_own_last_reading() {
        let mut t = DeltaTracker::new();
        t.delta("a.gguf", leitura(100.0, 10.0));
        let d = t.delta("b.gguf", leitura(7.0, 3.0));
        assert!((d.prompt_tokens - 7.0).abs() < f64::EPSILON);
    }

    /// Somar deltas num acumulador é campo a campo.
    #[test]
    fn adding_deltas_accumulates_field_by_field() {
        let mut acc = MetricCounters::default();
        acc.add(&leitura(100.0, 50.0));
        acc.add(&leitura(10.0, 5.0));
        assert!((acc.prompt_tokens - 110.0).abs() < f64::EPSILON);
        assert!((acc.predicted_tokens - 55.0).abs() < f64::EPSILON);
        assert!((acc.predicted_seconds - 5.5).abs() < 1e-9);
        assert!(!acc.is_zero());
        assert!(MetricCounters::default().is_zero());
    }
}
