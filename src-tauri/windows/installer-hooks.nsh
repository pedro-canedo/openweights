; Ganchos do instalador NSIS (Windows).
;
; Único gancho hoje: o atalho na Área de Trabalho nasce junto com a
; instalação.
;
; O instalador do Tauri já oferece esse atalho, mas como uma caixa de marcar
; na ÚLTIMA página — e ela só tem efeito em quem chega até ela e clica em
; "Concluir". Quem fecha a janela no X depois da barra de progresso termina
; com o app instalado e nenhum ícone à vista; o caminho vira procurar
; "OpenWeights" no menu Iniciar, que é exatamente o atrito que este gancho
; remove. Rodando no POSTINSTALL, o atalho existe assim que os arquivos
; existem, independentemente de como a janela seja fechada.
;
; `CreateOrUpdateDesktopShortcut` é a mesma função que o template do Tauri
; chama nas instalações silenciosas e passivas (as que também pulam a última
; página). Reusá-la é o que mantém as três regras dela de graça:
;
; - `/UPDATE` (que o updater do app sempre passa) não recria nada: um atalho
;   apagado de propósito não volta pelas costas de quem apagou;
; - `/NS` continua significando "instale sem atalhos";
; - um atalho da instalação antiga é reapontado, não duplicado.
;
; Nas instalações silenciosas e passivas o template já chamou a função uma
; vez, e este gancho a chama de novo: ela reescreve o mesmo `.lnk`, o que sai
; mais barato que replicar aqui a condição dele — e mantém este arquivo com um
; acoplamento só.
;
; A desinstalação não precisa de gancho: o template já apaga o
; `$DESKTOP\${PRODUCTNAME}.lnk` quando ele aponta para esta instalação.

!macro NSIS_HOOK_POSTINSTALL
  Call CreateOrUpdateDesktopShortcut
!macroend
