$ErrorActionPreference = 'Stop'

$root   = Split-Path $MyInvocation.MyCommand.Path
$vsix   = Join-Path $root "havakyrie-0.0.1.vsix"
$tmp    = Join-Path $root "__vsix_build"

# --- clean up any previous run ---
if (Test-Path $tmp)  { Remove-Item $tmp -Recurse -Force }
if (Test-Path $vsix) { Remove-Item $vsix -Force }

New-Item -ItemType Directory $tmp | Out-Null
New-Item -ItemType Directory "$tmp\extension" | Out-Null

# -------------------------------------------------------
# [Content_Types].xml
# -------------------------------------------------------
$contentTypes = @'
<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="json"         ContentType="application/json"/>
  <Default Extension="js"           ContentType="application/javascript"/>
  <Default Extension="map"          ContentType="application/json"/>
  <Default Extension="md"           ContentType="text/markdown"/>
  <Default Extension="vsixmanifest" ContentType="text/xml"/>
  <Default Extension="png"          ContentType="image/png"/>
</Types>
'@
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText("$tmp\[Content_Types].xml", $contentTypes, $utf8NoBom)

# -------------------------------------------------------
# extension.vsixmanifest
# -------------------------------------------------------
$manifest = @'
<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0"
  xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011"
  xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="havakyrie" Version="0.0.1" Publisher="havakyrie-publisher"/>
    <DisplayName>Havakyrie</DisplayName>
    <Description xml:space="preserve">Coding intelligence for Havakyrie (.hv files)</Description>
    <Tags>Havakyrie,hv</Tags>
    <Categories>Programming Languages</Categories>
    <GalleryFlags>Public</GalleryFlags>
    <Badges></Badges>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="^1.80.0"/>
      <Property Id="Microsoft.VisualStudio.Code.ExtensionDependencies" Value=""/>
      <Property Id="Microsoft.VisualStudio.Code.ExtensionPack" Value=""/>
      <Property Id="Microsoft.VisualStudio.Code.ExtensionKind" Value="ui,workspace"/>
      <Property Id="Microsoft.VisualStudio.Code.LocalizedLanguages" Value=""/>
      <Property Id="Microsoft.VisualStudio.Code.EnabledApiProposals" Value=""/>
    </Properties>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code"/>
  </Installation>
  <Dependencies/>
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest"
           Path="extension/package.json" Addressable="true"/>
    <Asset Type="Microsoft.VisualStudio.Services.VSIXPackage"
           d:Source="File" Path="|self|" Addressable="true"/>
  </Assets>
</PackageManifest>
'@
[System.IO.File]::WriteAllText("$tmp\extension.vsixmanifest", $manifest, $utf8NoBom)

# -------------------------------------------------------
# Compile TypeScript
# -------------------------------------------------------
Write-Host "Compiling TypeScript..."
npm --prefix "$root" run compile
if (-not $?) { throw "TypeScript compilation failed" }

# -------------------------------------------------------
# Copy extension files
# -------------------------------------------------------
Copy-Item "$root\package.json"               "$tmp\extension\package.json"
Copy-Item "$root\language-configuration.json" "$tmp\extension\language-configuration.json"

New-Item -ItemType Directory "$tmp\extension\out"      | Out-Null
New-Item -ItemType Directory "$tmp\extension\syntaxes" | Out-Null

Get-ChildItem "$root\out\*.js" | ForEach-Object {
    Copy-Item $_.FullName "$tmp\extension\out\$($_.Name)"
}
Copy-Item "$root\syntaxes\havakyrie.tmLanguage.json" "$tmp\extension\syntaxes\havakyrie.tmLanguage.json"
Copy-Item "$root\builtins.hvs"       "$tmp\extension\builtins.hvs"

# -------------------------------------------------------
# Zip → .vsix
# -------------------------------------------------------
Add-Type -Assembly System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($tmp, $vsix,
    [System.IO.Compression.CompressionLevel]::Optimal, $false)

Remove-Item $tmp -Recurse -Force

Write-Host "Created: $vsix  ($([Math]::Round((Get-Item $vsix).Length/1KB, 1)) KB)"
