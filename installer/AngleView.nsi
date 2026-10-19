; Angle View installer (NSIS 3). Build: makensis installer/AngleView.nsi
; Installs per user (no administrator rights needed) into %LOCALAPPDATA%\Programs\Angle View.

Unicode true
SetCompressor /SOLID lzma

!define APP "Angle View"
!define EXE "AngleView.exe"
!define PROGID "AngleView.Image"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\AngleView"
!ifndef VERSION
  !define VERSION "0.2.0"
!endif
!ifndef SRC
  !define SRC "..\target\x86_64-pc-windows-gnu\release"
!endif

Name "${APP}"
OutFile "AngleView-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "Software\AngleView" "InstallDir"
RequestExecutionLevel user
BrandingText "${APP} ${VERSION}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

!define MUI_ICON "..\assets\icon.ico"
!define MUI_UNICON "..\assets\icon.ico"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Запустить ${APP}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=${LANG_RUSSIAN} "ProductName" "${APP}"
VIAddVersionKey /LANG=${LANG_RUSSIAN} "FileDescription" "Установка ${APP}"
VIAddVersionKey /LANG=${LANG_RUSSIAN} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_RUSSIAN} "ProductVersion" "${VERSION}"

; Registers the program for one extension: listed in "Open with" and in Default apps.
!macro Assoc EXT
  WriteRegStr HKCU "Software\Classes\.${EXT}\OpenWithProgids" "${PROGID}" ""
  WriteRegStr HKCU "Software\AngleView\Capabilities\FileAssociations" ".${EXT}" "${PROGID}"
  WriteRegStr HKCU "Software\Classes\Applications\${EXE}\SupportedTypes" ".${EXT}" ""
!macroend

; Makes the program the handler where Windows still allows it without the user's click
; (extensions that have no choice saved in Settings yet).
!macro MakeDefault EXT
  WriteRegStr HKCU "Software\Classes\.${EXT}" "" "${PROGID}"
!macroend

!macro Unassoc EXT
  DeleteRegValue HKCU "Software\Classes\.${EXT}\OpenWithProgids" "${PROGID}"
  ReadRegStr $0 HKCU "Software\Classes\.${EXT}" ""
  StrCmp $0 "${PROGID}" 0 +2
    DeleteRegValue HKCU "Software\Classes\.${EXT}" ""
  DeleteRegKey /ifempty HKCU "Software\Classes\.${EXT}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${EXT}"
!macroend

!macro ForEachExt MACRO
  !insertmacro ${MACRO} jpg
  !insertmacro ${MACRO} jpeg
  !insertmacro ${MACRO} jpe
  !insertmacro ${MACRO} jfif
  !insertmacro ${MACRO} png
  !insertmacro ${MACRO} apng
  !insertmacro ${MACRO} gif
  !insertmacro ${MACRO} webp
  !insertmacro ${MACRO} bmp
  !insertmacro ${MACRO} dib
  !insertmacro ${MACRO} tif
  !insertmacro ${MACRO} tiff
  !insertmacro ${MACRO} ico
  !insertmacro ${MACRO} jxl
  !insertmacro ${MACRO} svg
  !insertmacro ${MACRO} svgz
  !insertmacro ${MACRO} tga
  !insertmacro ${MACRO} qoi
!macroend

Section "${APP}" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRC}\${EXE}"
  File "/oname=README.md" "..\README.md"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\AngleView" "InstallDir" "$INSTDIR"

  ; Program identity and open command.
  WriteRegStr HKCU "Software\Classes\${PROGID}" "" "Изображение (${APP})"
  WriteRegStr HKCU "Software\Classes\${PROGID}\DefaultIcon" "" '"$INSTDIR\${EXE}",0'
  WriteRegStr HKCU "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKCU "Software\Classes\Applications\${EXE}" "FriendlyAppName" "${APP}"
  WriteRegStr HKCU "Software\Classes\Applications\${EXE}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\${EXE}" "" "$INSTDIR\${EXE}"

  ; Default apps entry, so all formats can be switched to the program at once.
  WriteRegStr HKCU "Software\AngleView\Capabilities" "ApplicationName" "${APP}"
  WriteRegStr HKCU "Software\AngleView\Capabilities" "ApplicationDescription" "Минималистичный просмотрщик изображений"
  WriteRegStr HKCU "Software\AngleView\Capabilities" "ApplicationIcon" '"$INSTDIR\${EXE}",0'
  WriteRegStr HKCU "Software\RegisteredApplications" "AngleView" "Software\AngleView\Capabilities"
  !insertmacro ForEachExt Assoc

  ; Start menu and uninstall entry.
  CreateDirectory "$SMPROGRAMS"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" '"$INSTDIR\${EXE}",0'
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Удаление фона (нейросеть, 89 МБ)" SecBg
  SetOutPath "$INSTDIR"
  File "..\models\birefnet-lite.safetensors"
SectionEnd

Section "Ярлык на рабочем столе" SecDesktop
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}"
SectionEnd

Section "Открывать все изображения в Angle View" SecDefault
  !insertmacro ForEachExt MakeDefault
  ; Windows 10/11 keep the final choice to the user: open the program's page in
  ; Default apps, where one "Set default" button assigns every format.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ExecShell "open" "ms-settings:defaultapps?registeredAppUser=AngleView"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "Сама программа."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecBg} "Файл нейросети BiRefNet для пункта «Удалить фон» и опции в экспорте. Работает без интернета."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Значок на рабочем столе."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDefault} "Назначить ${APP} программой по умолчанию для JPEG, PNG, GIF, WebP, BMP, TIFF, ICO, JPEG XL, SVG, TGA и QOI. В конце откроются «Приложения по умолчанию» Windows: там нужно один раз нажать «Установить по умолчанию»."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function .onInstSuccess
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
FunctionEnd

Section "Uninstall"
  !insertmacro ForEachExt Unassoc
  DeleteRegKey HKCU "Software\Classes\${PROGID}"
  DeleteRegKey HKCU "Software\Classes\Applications\${EXE}"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\${EXE}"
  DeleteRegValue HKCU "Software\RegisteredApplications" "AngleView"
  DeleteRegKey HKCU "Software\AngleView"
  DeleteRegKey HKCU "${UNINST_KEY}"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\birefnet-lite.safetensors"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd
