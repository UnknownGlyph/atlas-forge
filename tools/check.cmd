@echo off
setlocal
python "%~dp0i18n_catalog.py" || exit /b 1
python -B -m unittest discover -s "%~dp0." -p test_i18n_catalog.py || exit /b 1
rem What continuous integration runs, locally on Windows: build, tests, clippy.
rem   tools\check.cmd          the usual checks
rem   tools\check.cmd soak     also the long random authoring run (minutes)
call "%~dp0cargo-msvc.cmd" build --locked --workspace --release --all-targets || exit /b 1
call "%~dp0cargo-msvc.cmd" test --locked --workspace --release --no-fail-fast || exit /b 1
call "%~dp0cargo-msvc.cmd" clippy --locked --workspace --release --all-targets || exit /b 1
if "%1"=="soak" (
  set ATLAS_SOAK_SEEDS=32
  set ATLAS_SOAK_STEPS=1500
  call "%~dp0cargo-msvc.cmd" test --locked --release -p map_io --test soak || exit /b 1
)
echo ALL CHECKS PASSED
