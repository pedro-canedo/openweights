; Ganchos do instalador NSIS (Windows).
;
; Único gancho hoje: o atalho na Área de Trabalho nasce junto com a
; instalação — inclusive quando ela chega por atualização.
;
; O instalador do Tauri já oferece esse atalho, mas como uma caixa de marcar
; na ÚLTIMA página, e ela só tem efeito em quem chega até lá e clica em
; "Concluir". Quem fecha a janela no X depois da barra de progresso termina
; com o app instalado e nenhum ícone à vista. Rodando no POSTINSTALL, o
; atalho existe assim que os arquivos existem, seja qual for o caminho.
;
; O caso que faltava é o da ATUALIZAÇÃO. O `CreateOrUpdateDesktopShortcut` do
; template sai sem fazer nada quando `$UpdateMode = 1`, e o updater do app
; sempre passa `/UPDATE` — então quem instalou uma vez e desde então só
; atualiza nunca ganha o atalho, por mais versões que passem. A regra do
; template está certa pelo motivo dela (um atalho apagado de propósito não
; deve voltar pelas costas de quem apagou), mas ela não distingue "o usuário
; apagou" de "nunca existiu". Este gancho distingue, com uma marca no
; registro ao lado das chaves de desinstalação:
;
; - sem a marca, esta instalação nunca teve a chance de ganhar um atalho, e
;   criá-lo agora não contraria escolha nenhuma;
; - com a marca, a decisão volta a ser do template, que respeita quem apagou.
;
; `/NS` ("instale sem atalhos") continua valendo em qualquer modo, e o
; reaproveitamento do `CreateOrUpdateDesktopShortcut` é o que mantém de graça
; a migração do nome antigo do binário e o AppUserModelId do `.lnk`.
;
; A desinstalação não precisa de gancho: o template já apaga o
; `$DESKTOP\${PRODUCTNAME}.lnk` quando ele aponta para esta instalação, e a
; marca vai junto com a chave de desinstalação inteira.

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  Push $1
  ${If} $NoShortcutMode = 0
    ReadRegStr $0 SHCTX "${UNINSTKEY}" "DesktopShortcutCreated"
    ${If} $0 == ""
      ; Primeira vez: cria mesmo em atualização, emprestando o modo normal
      ; para a função do template em vez de duplicar o que ela faz.
      StrCpy $1 $UpdateMode
      StrCpy $UpdateMode 0
      Call CreateOrUpdateDesktopShortcut
      StrCpy $UpdateMode $1
      WriteRegStr SHCTX "${UNINSTKEY}" "DesktopShortcutCreated" "1"
    ${Else}
      Call CreateOrUpdateDesktopShortcut
    ${EndIf}
  ${EndIf}
  Pop $1
  Pop $0
!macroend
