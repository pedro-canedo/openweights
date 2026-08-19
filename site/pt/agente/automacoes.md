# Automações

Uma automação é um pedido que roda sozinho de tempos em tempos, mesmo com você
longe.

**Configurações → Automações → Nova automação**:

| Campo | O que é |
|---|---|
| Nome | Como ela aparece na lista |
| O que fazer | O pedido, exatamente como você digitaria no chat |
| Pasta do projeto | Onde ela roda; opcional |
| Modelo | Ou deixe vazio para o primeiro da sua biblioteca |
| Autorização e modo de trabalho | Os mesmos níveis de uma execução normal |
| De quanto em quanto tempo | A cada hora, a cada 4 horas, diária, semanal, um intervalo em minutos, ou só quando você mandar |

## O que "rodar sem ninguém olhando" significa de verdade

Ninguém está olhando quando uma automação dispara. Então o harness faz a coisa
chata e correta: em qualquer nível abaixo do automático, ela **para no primeiro
pedido de confirmação e espera por você**. A lista mostra *Parada, esperando
você*, e abrir a execução deixa você responder para ela continuar.

O mesmo vale para as outras paradas: *Parou no limite de passos*, *Parou após
erros repetidos*.

::: warning O relógio mora dentro do app
Com o OpenWeights fechado, nada dispara. Na hora marcada o motor de IA sobe
sozinho — mas o app precisa estar aberto para o relógio andar.
:::

**Rodar agora** dispara uma na hora, que também é como você testa o pedido antes
de confiá-lo a um horário. Desligar uma automação a mantém na lista sem executar.

Toda execução de automação cai em **Atividade** como qualquer outra, com a
trilha completa.
