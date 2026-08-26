# MCManager - script d'installation Windows.
# Usage :
#   iex (irm mcmanager.pages.dev/install.ps1)
#   iex "& { $(irm mcmanager.pages.dev/install.ps1) } -Silent"    # sans interaction
#   iex "& { $(irm mcmanager.pages.dev/install.ps1) } -Uninstall" # desinstalle
#
# Telecharge et lance le veritable installateur Inno Setup (mcmanager-*-setup.exe)
# depuis la derniere release GitHub - c'est cet installateur qui cree le raccourci
# dans le menu Demarrer et enregistre la desinstallation dans "Applications
# installees" (comportement natif d'Inno Setup, pas quelque chose que ce script
# doit recreer a la main).
param(
    [switch]$Silent,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$repo = "yo-le-zz/MCmanager"

if ($Uninstall) {
    Write-Host "Recherche de MCManager dans les applications installees..."
    $uninstallKey = Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall","HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall" -ErrorAction SilentlyContinue |
        Get-ItemProperty | Where-Object { $_.DisplayName -like "MCManager*" } | Select-Object -First 1
    if (-not $uninstallKey) {
        Write-Error "MCManager ne semble pas installe via l'installateur .exe (rien trouve dans le registre)."
        exit 1
    }
    Write-Host "Lancement du desinstalleur..."
    Start-Process -FilePath $uninstallKey.UninstallString -Wait
    Write-Host "MCManager desinstalle."
    exit 0
}

Write-Host "Recherche de la derniere version de MCManager..."
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest"
$asset = $release.assets | Where-Object { $_.name -like "*setup.exe" } | Select-Object -First 1

if (-not $asset) {
    Write-Error "Aucun installateur .exe trouve dans la derniere release GitHub ($repo)."
    exit 1
}

$tmp = Join-Path $env:TEMP $asset.name
Write-Host "Telechargement de $($asset.name)..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp

Write-Host "Lancement de l'installateur..."
if ($Silent) {
    Start-Process -FilePath $tmp -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES" -Wait
} else {
    Start-Process -FilePath $tmp -Wait
}

Write-Host ""
Write-Host "Termine. MCManager est disponible dans le menu Demarrer."
Write-Host "Desinstallation : Parametres Windows -> Applications installees -> MCManager -> Desinstaller"
Write-Host "                  (ou relancez ce script avec -Uninstall)"
