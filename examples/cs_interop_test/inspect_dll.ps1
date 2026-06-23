$path = "D:\repository\new_language\examples\cs_interop_test\ArrowBridge.dll"
$data = [System.IO.File]::ReadAllBytes($path)

$peOff = [System.BitConverter]::ToUInt32($data, 0x3C)
Write-Host "e_lfanew = 0x$($peOff.ToString('X'))"

$peSig = [System.BitConverter]::ToUInt32($data, $peOff)
Write-Host "PE sig = 0x$($peSig.ToString('X8'))"

$coff = $peOff + 4
$numSections = [System.BitConverter]::ToUInt16($data, $coff + 2)
$optHdrSize  = [System.BitConverter]::ToUInt16($data, $coff + 16)
$optHdr = $coff + 20
$magic  = [System.BitConverter]::ToUInt16($data, $optHdr)
Write-Host "magic = 0x$($magic.ToString('X4'))"

$isDirs = if ($magic -eq 0x20B) { 112 } else { 96 }
$dataDirsOff = $optHdr + $isDirs
$cliRva = [System.BitConverter]::ToUInt32($data, $dataDirsOff + 14 * 8)
Write-Host "CLI RVA = 0x$($cliRva.ToString('X'))"

$sectOff = $optHdr + $optHdrSize
Write-Host "numSections = $numSections"
$sections = @()
for ($i = 0; $i -lt $numSections; $i++) {
    $sh = $sectOff + $i * 40
    $nameBytes = $data[$sh..($sh+7)]
    $name = [System.Text.Encoding]::ASCII.GetString($nameBytes).TrimEnd([char]0)
    $virtSz  = [System.BitConverter]::ToUInt32($data, $sh + 8)
    $virtRva = [System.BitConverter]::ToUInt32($data, $sh + 12)
    $rawOff  = [System.BitConverter]::ToUInt32($data, $sh + 20)
    Write-Host "  [$i] name=$name virt=0x$($virtRva.ToString('X')) vsz=$virtSz raw=0x$($rawOff.ToString('X'))"
    $sections += @{ virt=$virtRva; vsz=$virtSz; raw=$rawOff }
}

function Resolve-RVA($rva) {
    foreach ($s in $sections) {
        if ($rva -ge $s.virt -and $rva -lt ($s.virt + [Math]::Max($s.vsz, 1))) {
            return $rva - $s.virt + $s.raw
        }
    }
    return -1
}

$cliOff = Resolve-RVA $cliRva
Write-Host "CLI header file offset = 0x$($cliOff.ToString('X'))"

$metaRva = [System.BitConverter]::ToUInt32($data, $cliOff + 8)
Write-Host "Metadata RVA = 0x$($metaRva.ToString('X'))"

$metaOff = Resolve-RVA $metaRva
Write-Host "Metadata file offset = 0x$($metaOff.ToString('X'))"

$bsjb = [System.BitConverter]::ToUInt32($data, $metaOff)
Write-Host "BSJB at metadata = 0x$($bsjb.ToString('X8')) (expect 0x424A5342)"
