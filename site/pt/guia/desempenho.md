# Desempenho no chat

Uma resposta lenta pode estar esperando na fila, carregando um modelo,
processando um prompt grande ou gerando tokens. O OpenWeights mostra essas
fases separadamente para que um ajuste tenha uma causa verificável.

Depois de uma resposta, abra **Detalhes da execução**. Espera e duração total
são medidas pelo app; tokens, cache e velocidade vêm do motor. Quando ele não
informa uma métrica, o app mostra “Não informado”. O primeiro token pode ser de
raciocínio: o primeiro texto da resposta tem sua própria medição.

Para comparar configurações, abra **Servidor Local → Desempenho** e escolha um
modelo local. O teste faz um aquecimento e três medições para o perfil atual e
para o candidato. Uma diferença só é apresentada como ganho quando não se
mistura com a variação das amostras e prompt e latência não regridem mais de 5%.
Variação acima de 10% torna o resultado inconclusivo.

Defina um contexto explícito antes de comparar. Modelo, quantização, contexto,
esforço de raciocínio e extras personalizados são preservados. O teste exige
servidor ocioso e pausa novos trabalhos internos. A API local fica indisponível
enquanto as medições rodam em um servidor isolado. Evite clientes externos e
outros programas usando a GPU. Se não for possível verificar a atividade dos
slots, o teste não começa.

A memória disponível é a soma informada pela listagem de dispositivos do motor
após o aquecimento. Não é o pico de uso nem a memória alocada pelo modelo;
drivers e memória compartilhada podem afetar a leitura. Dados ausentes ficam
indisponíveis. As verificações de variação não detectam toda interferência
externa possível.

Escolha **Aplicar candidato** ou **Manter atual**. Depois de aplicar, é possível
**Restaurar configuração anterior**. Medir não grava perfis experimentais;
cancelamento ou erro restaura a disponibilidade do servidor, se estava ligado.
É possível sair da tela e voltar para cancelar o teste.

O histórico é virtualizado e recebe atualizações visuais a cada 50 ms, com
conclusão, erro e cancelamento imediatos. O destaque de código é feito ao
terminar. Parar preserva a resposta parcial e as métricas registradas.

No cenário sintético Windows com 500 mensagens e 100 fragmentos/s, a mediana
de CPU de renderização React caiu de 2.688,6 ms para 241,0 ms em três rodadas
(91,0%). Isso mede a interface em desenvolvimento, não ganho de tokens/s.
Veja a [metodologia e limitações](https://github.com/pedro-canedo/openweights/blob/main/docs/performance-0.17.0.md).
