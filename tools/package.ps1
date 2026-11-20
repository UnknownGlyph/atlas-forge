# Builds a release, checks it with its own self-test, and makes a portable
# zip for Windows:
#   powershell -ExecutionPolicy Bypass -File tools\package.ps1
#   -> dist\AtlasForge-<version>-windows-x64.zip
# -NoBuild packages what is already built (in target\package).
#
# The build goes to target\package with the source and cargo folders
# remapped (--remap-path-prefix) and only the PDB's file name kept in the
# programs, so they carry no path of the computer that built them; the
# script checks that before zipping.
param([switch]$NoBuild)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$target = Join-Path $root "target\package"
if (-not $NoBuild) {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
    $flags = @("--remap-path-prefix=$root=atlas-forge", "--remap-path-prefix=$cargoHome=cargo", "-Clink-arg=/PDBALTPATH:%_PDB%")
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f
    $env:CARGO_TARGET_DIR = $target
    & "$PSScriptRoot\cargo-msvc.cmd" build --locked --release -p map_app -p map_api -p map_ai
    $code = $LASTEXITCODE
    Remove-Item Env:\CARGO_ENCODED_RUSTFLAGS, Env:\CARGO_TARGET_DIR
    if ($code -ne 0) { throw "构建失败" }
}
$bin = Join-Path $target "release"
$name = "AtlasForge-$version-windows-x64"
$dist = Join-Path $root "dist"
$run = Join-Path $target ("stage-" + [Guid]::NewGuid().ToString("N"))
$out = Join-Path $run $name
New-Item -ItemType Directory -Force (Join-Path $out "示例"), (Join-Path $out "fonts") | Out-Null
$programs = "atlas.exe", "atlas-query.exe", "atlas-mcp.exe", "atlas-agent.exe", "atlas-script.exe"
foreach ($exe in $programs) {
    if (-not (Test-Path "$bin\$exe")) { throw "没有 $bin\$exe" }
    Copy-Item "$bin\$exe" $out
}
# the default font, with its licence
Copy-Item "$root\assets\fonts\NotoSansSC-Regular.ttf", "$root\assets\fonts\OFL.txt" (Join-Path $out "fonts")
Copy-Item "$root\LICENSE" (Join-Path $out "LICENSE.txt")
Copy-Item "$root\docs\USER_GUIDE.md" (Join-Path $out "使用说明.md")
Copy-Item "$root\docs\USER_GUIDE.en.md" (Join-Path $out "USER_GUIDE.en.md")
Copy-Item "$root\docs\CLIMATE.md" (Join-Path $out "CLIMATE.md")
Copy-Item "$root\docs\API.md", "$root\docs\PLUGINS.md" $out
Copy-Item "$root\docs\WORLD_BOOK.md" $out
Copy-Item "$root\docs\CREATION_WORKSPACES.md", "$root\docs\ASTRONOMY.md" $out
Copy-Item "$root\docs\tutorial" (Join-Path $out "tutorial") -Recurse
Copy-Item "$root\docs\examples\create-city.atlas-plugin.json" (Join-Path $out "示例")
Copy-Item "$root\crates\map_io\tests\fixtures\v1_world.atlas" (Join-Path $out "示例\示例世界.atlas")
# no path or name of this computer in the programs
$privatePaths = @($root, $env:USERPROFILE) | Where-Object { $_ -and $_.Length -ge 3 }
$byteText = [Text.Encoding]::GetEncoding(28591)
$gaia = [IO.File]::ReadAllText((Join-Path $root 'assets/astronomy/gaia-dr3-bright.csv')).Replace("`r`n", "`n")
$knownGaia = $byteText.GetString([Text.Encoding]::UTF8.GetBytes($gaia))
foreach ($exe in $programs) {
    $bytes = [IO.File]::ReadAllBytes((Join-Path $out $exe))
    $text = $byteText.GetString($bytes)
    $wide = [Text.Encoding]::Unicode.GetString($bytes)
    foreach ($path in $privatePaths) {
        foreach ($spelling in @($path, $path.Replace([char]92, [char]47))) {
            $utf8 = $byteText.GetString([Text.Encoding]::UTF8.GetBytes($spelling))
            if ($text.IndexOf($utf8, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or $wide.IndexOf($spelling, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw "$exe 中含有本机路径"
            }
        }
    }
    if ($env:USERNAME -and $env:USERNAME.Length -ge 3) {
        # Exact embedded public catalog bytes can contain a decimal value
        # whose digits coincide with a local account name. Check every byte
        # outside that known block and keep the path check over the whole file.
        $accountText = if ($exe -eq "atlas.exe") { $text.Replace($knownGaia, "") } else { $text }
        $account = $byteText.GetString([Text.Encoding]::UTF8.GetBytes($env:USERNAME))
        if ($accountText.IndexOf($account, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or $wide.IndexOf($env:USERNAME, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "$exe 中含有本机用户名"
        }
    }
}
# the package checks itself: a world, a partition, roads, saving, exports
$check = Join-Path $run "self-test"
$p = Start-Process -FilePath (Join-Path $out "atlas.exe") -ArgumentList "--self-test", "`"$check`"" -WindowStyle Hidden -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "自检失败（退出码 $($p.ExitCode)）" }
$report = Get-Content (Join-Path $check "自检报告.txt") -Encoding UTF8
$report | Write-Host
if (-not ($report -match "随附的 Noto Sans SC")) { throw "自检没有用到随附的字体" }
$zip = Join-Path $dist "$name.zip"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$stagedZip = Join-Path $run "$name.zip"
Compress-Archive -LiteralPath $out -DestinationPath $stagedZip
Move-Item -LiteralPath $stagedZip -Destination $zip -Force
Write-Host "已生成 $zip（$([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB）"
