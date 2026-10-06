#!/usr/bin/env bash
#
# build-release.sh — genera los binarios distribuibles de Rustidian.
#
# Salidas (según la plataforma objetivo):
#   * .deb            Debian / Ubuntu / derivados   (cargo-deb)
#   * .rpm            Fedora / RHEL / derivados      (cargo-generate-rpm)
#   * .zip + .exe     Windows x86_64                 (cross-compile mingw-w64)
#   * .dmg / .tar.gz  macOS                          (local en macOS o vía GitHub Actions)
#
# Opcionalmente publica todo en un GitHub Release usando `gh` (debe estar
# autenticado: `gh auth status`).
#
# Uso:
#   ./scripts/build-release.sh [opciones]
#
# Opciones:
#   --all             Genera todos los objetivos posibles (por defecto).
#   --deb             Genera el paquete .deb.
#   --rpm             Genera el paquete .rpm.
#   --windows         Genera el .exe / .zip de Windows (cross-compile local).
#   --macos           Genera el artefacto de macOS (local o vía CI).
#   --windows-ci      Fuerza compilar Windows en GitHub Actions.
#   --macos-ci        Fuerza compilar macOS en GitHub Actions.
#   --graph           Compila con la feature `graph` (experimental, beta).
#   --version <v>     Sobrescribe la versión (por defecto: la de Cargo.toml).
#   --tag <tag>       Tag de GitHub Release (por defecto: v<version>).
#   --no-release      No publica en GitHub; solo deja los archivos en --out.
#   --no-tools        No instala automáticamente cargo-deb/cargo-generate-rpm.
#   --watch-ci        Espera a que termine el workflow de CI lanzado.
#   -o, --out <dir>   Directorio de salida (por defecto: target/distrib).
#   -y, --yes         No pregunta (crea/publica el release sin confirmar).
#   -h, --help        Muestra esta ayuda.
#
set -o pipefail

# ── Presentación ─────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
    C_RESET=$'\033[0m'; C_BOLD=$'\033[1m'; C_DIM=$'\033[2m'
    C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_BLUE=$'\033[34m'
else
    C_RESET=""; C_BOLD=""; C_DIM=""; C_RED=""; C_GREEN=""; C_YELLOW=""; C_BLUE=""
fi
log()  { printf '%s==>%s %s\n' "$C_BLUE$C_BOLD" "$C_RESET" "$*"; }
step() { printf '%s  •%s %s\n' "$C_DIM" "$C_RESET" "$*"; }
ok()   { printf '%s  ✓%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
warn() { printf '%swarning:%s %s\n' "$C_YELLOW$C_BOLD" "$C_RESET" "$*" >&2; }
err()  { printf '%serror:%s %s\n' "$C_RED$C_BOLD" "$C_RESET" "$*" >&2; }
die()  { err "$*"; exit 1; }

# ── Rutas / entorno ──────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT" || die "no se pudo entrar a $ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

PKG="rustidian-ui"
HOST_OS="$(uname -s)"

# ── Valores por defecto ──────────────────────────────────────────────────────
DO_DEB=0; DO_RPM=0; DO_WINDOWS=0; DO_MACOS=0
ANY_TARGET=0
WINDOWS_CI=0; MACOS_CI=0
GRAPH=0
VERSION=""
TAG=""
OUT_DIR="$ROOT/target/distrib"
PUBLISH=1
AUTO_TOOLS=1
WATCH_CI=0
ASSUME_YES=0

usage() { sed -n '2,34p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --all)        DO_DEB=1; DO_RPM=1; DO_WINDOWS=1; DO_MACOS=1; ANY_TARGET=1 ;;
        --deb)        DO_DEB=1; ANY_TARGET=1 ;;
        --rpm)        DO_RPM=1; ANY_TARGET=1 ;;
        --windows)    DO_WINDOWS=1; ANY_TARGET=1 ;;
        --macos)      DO_MACOS=1; ANY_TARGET=1 ;;
        --windows-ci) DO_WINDOWS=1; WINDOWS_CI=1; ANY_TARGET=1 ;;
        --macos-ci)   DO_MACOS=1; MACOS_CI=1; ANY_TARGET=1 ;;
        --graph)      GRAPH=1 ;;
        --version)    VERSION="${2:?--version requiere un valor}"; shift ;;
        --tag)        TAG="${2:?--tag requiere un valor}"; shift ;;
        --no-release) PUBLISH=0 ;;
        --no-tools)   AUTO_TOOLS=0 ;;
        --watch-ci)   WATCH_CI=1 ;;
        -o|--out)     OUT_DIR="${2:?--out requiere un valor}"; shift ;;
        -y|--yes)     ASSUME_YES=1 ;;
        -h|--help)    usage; exit 0 ;;
        *)            die "opción desconocida: $1 (usa --help)" ;;
    esac
    shift
done

# Sin objetivos explícitos -> todos.
if [[ $ANY_TARGET -eq 0 ]]; then
    DO_DEB=1; DO_RPM=1; DO_WINDOWS=1; DO_MACOS=1
fi

# ── Versión / tag ────────────────────────────────────────────────────────────
if [[ -z "$VERSION" ]]; then
    VERSION="$(grep -m1 '^version' "$PKG/Cargo.toml" | cut -d'"' -f2)"
fi
[[ -n "$VERSION" ]] || die "no se pudo detectar la versión en $PKG/Cargo.toml"
if [[ -z "$TAG" ]]; then
    TAG="v$VERSION"
fi

FEATURE_ARGS=()
[[ $GRAPH -eq 1 ]] && FEATURE_ARGS=(--features graph)

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

FAILED=()
PRODUCED=()

# ── Utilidades ───────────────────────────────────────────────────────────────
have() { command -v "$1" >/dev/null 2>&1; }

cargo_subcmd_available() {
    # cargo-deb/cargo-generate-rpm instalan un binario `cargo-<sub>` en el PATH.
    have "$1"
}

ensure_cargo_tool() {
    local crate="$1" bin="$2"
    if cargo_subcmd_available "$bin"; then
        return 0
    fi
    if [[ $AUTO_TOOLS -eq 0 ]]; then
        err "falta '$bin'. Instálalo con: cargo install --locked $crate"
        return 1
    fi
    log "Instalando $crate (cargo install --locked $crate)…"
    cargo install --locked "$crate" || { err "no se pudo instalar $crate"; return 1; }
    cargo_subcmd_available "$bin" || { err "'$bin' sigue sin estar disponible"; return 1; }
}

rust_target_installed() {
    local target="$1"
    if have rustup && rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        return 0
    fi
    [[ -d "$(rustc --print sysroot 2>/dev/null)/lib/rustlib/$target" ]]
}

ensure_rust_target() {
    local target="$1"
    rust_target_installed "$target" && return 0
    if ! have rustup; then
        err "el target '$target' no está instalado y no hay rustup."
        err "Instala rustup (https://rustup.rs) y ejecuta: rustup target add $target"
        return 1
    fi
    log "Añadiendo target de Rust: $target"
    rustup target add "$target" || { err "no se pudo añadir el target $target"; return 1; }
}

mingw_available() { have x86_64-w64-mingw32-gcc || have x86_64-w64-mingw32-gcc-win32; }

windows_can_build_local() { mingw_available && rust_target_installed x86_64-pc-windows-gnu; }

install_hint() {
    case "$1" in
        mingw)
            cat >&2 <<'EOF'
Instala el toolchain de mingw-w64 para compilar para Windows:
  Arch:    sudo pacman -S --needed mingw-w64-gcc
  Debian:  sudo apt install -y gcc-mingw-w64-x86-64
  Fedora:  sudo dnf install -y mingw64-gcc
EOF
            ;;
    esac
}

# ── Compilación local (Linux/macOS) ──────────────────────────────────────────
build_host() {
    local extra=""
    [[ $GRAPH -eq 1 ]] && extra=" (con feature graph)"
    log "Compilando $PKG $VERSION$extra para el host…"
    cargo build --release -p "$PKG" ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"}
}

# ── .deb ─────────────────────────────────────────────────────────────────────
build_deb() {
    ensure_cargo_tool cargo-deb cargo-deb || return 1
    log "Generando paquete .deb…"
    cargo deb -p "$PKG" --no-build ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"} --output "$OUT_DIR"
    local f
    for f in "$OUT_DIR"/*.deb; do [[ -e "$f" ]] && PRODUCED+=("$f"); done
    return 0
}

# ── .rpm ─────────────────────────────────────────────────────────────────────
build_rpm() {
    ensure_cargo_tool cargo-generate-rpm cargo-generate-rpm || return 1
    log "Generando paquete .rpm…"
    if [[ $GRAPH -eq 1 ]]; then
        warn "cargo-generate-rpm no acepta --features: el .rpm se generará SIN la feature 'graph'"
    fi
    cargo generate-rpm -p "$PKG" || return 1
    local src="$ROOT/target/generate-rpm"
    local f
    shopt -s nullglob
    for f in "$src"/*.rpm; do
        cp -f "$f" "$OUT_DIR/"
        PRODUCED+=("$OUT_DIR/$(basename "$f")")
    done
    shopt -u nullglob
    return 0
}

# ── Windows (.exe cross-compile + .zip) ──────────────────────────────────────
build_windows() {
    if ! mingw_available; then
        err "no se encontró el compilador de mingw-w64 (x86_64-w64-mingw32-gcc)"
        install_hint mingw
        return 1
    fi
    ensure_rust_target x86_64-pc-windows-gnu || return 1

    log "Compilando para Windows (x86_64-pc-windows-gnu)…"
    cargo build --release -p "$PKG" \
        --target x86_64-pc-windows-gnu ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"} || return 1

    local exe="$ROOT/target/x86_64-pc-windows-gnu/release/$PKG.exe"
    [[ -f "$exe" ]] || { err "no se generó $exe"; return 1; }

    local stage="$OUT_DIR/rustidian-$VERSION-windows-x86_64"
    rm -rf "$stage"; mkdir -p "$stage"
    cp "$exe" "$stage/"
    cp "$ROOT/LICENSE" "$stage/" 2>/dev/null || true
    cp "$ROOT/README.md" "$stage/" 2>/dev/null || true

    local zip="$OUT_DIR/rustidian-$VERSION-windows-x86_64.zip"
    rm -f "$zip"
    if have zip; then
        ( cd "$OUT_DIR" && zip -q -r "$zip" "$(basename "$stage")" )
        rm -rf "$stage"
    else
        warn "no hay 'zip'; se deja la carpeta sin comprimir"
        zip="$stage"
    fi
    PRODUCED+=("$zip")
    return 0
}

# ── macOS ────────────────────────────────────────────────────────────────────
make_app_bundle() {
    # $1 = binario, $2 = directorio destino .app
    local bin="$1" app="$2"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin" "$app/Contents/MacOS/rustidian-ui"
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Rustidian</string>
    <key>CFBundleDisplayName</key><string>Rustidian</string>
    <key>CFBundleIdentifier</key><string>com.github.hu-tao128.rustidian</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleExecutable</key><string>rustidian-ui</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
}

build_macos_local() {
    local arch
    case "$(uname -m)" in
        arm64|aarch64) arch="arm64" ;;
        x86_64)        arch="x86_64" ;;
        *)             arch="$(uname -m)" ;;
    esac
    log "Compilando para macOS ($arch)…"
    cargo build --release -p "$PKG" ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"} || return 1

    local app="$OUT_DIR/Rustidian.app"
    make_app_bundle "$ROOT/target/release/$PKG" "$app" || return 1

    if have hdiutil; then
        local dmg="$OUT_DIR/rustidian-$VERSION-macos-$arch.dmg"
        rm -f "$dmg"
        hdiutil create -volname "Rustidian $VERSION" -srcfolder "$app" \
            -ov -format UDZO "$dmg" >/dev/null || return 1
        PRODUCED+=("$dmg")
    fi

    local targz="$OUT_DIR/rustidian-$VERSION-macos-$arch.tar.gz"
    tar -C "$OUT_DIR" -czf "$targz" "Rustidian.app" || return 1
    PRODUCED+=("$targz")
    return 0
}

dispatch_ci() {
    local platforms="$1"   # p. ej. "macos,windows"
    have gh || { err "se necesita 'gh' para compilar en CI"; return 1; }

    local wf="ci-release.yml"
    if ! gh workflow view "$wf" >/dev/null 2>&1; then
        err "el workflow '$wf' no existe en el repositorio remoto."
        err "Haz commit y push de .github/workflows/$wf a la rama por defecto e inténtalo de nuevo."
        return 1
    fi

    if [[ $PUBLISH -eq 1 ]]; then
        ensure_release || return 1
    fi

    local branch
    branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)"

    log "Lanzando GitHub Actions para: $platforms"
    gh workflow run "$wf" --ref "$branch" \
        -f "tag=$TAG" -f "version=$VERSION" \
        -f "graph=$([[ $GRAPH -eq 1 ]] && echo true || echo false)" \
        -f "platforms=$platforms" || return 1
    ok "workflow '$wf' lanzado (ref: $branch, tag: $TAG, plataformas: $platforms)"

    if [[ $WATCH_CI -eq 1 ]]; then
        sleep 5
        local run_id
        run_id="$(gh run list --workflow "$wf" --limit 1 --json databaseId --jq '.[0].databaseId' 2>/dev/null)"
        if [[ -n "$run_id" ]]; then
            gh run watch "$run_id" --exit-status || return 1
        else
            warn "no se pudo seguir el run; revisa 'gh run list'"
        fi
    else
        step "sigue el progreso con: gh run watch \$(gh run list --workflow $wf --limit 1 --json databaseId --jq '.[0].databaseId')"
    fi
    return 0
}

# ── Publicación en GitHub Releases ───────────────────────────────────────────
RELEASE_READY=0
ensure_release() {
    have gh || { err "se necesita 'gh' para publicar el release"; return 1; }
    if [[ $RELEASE_READY -eq 1 ]]; then
        return 0
    fi
    if gh release view "$TAG" >/dev/null 2>&1; then
        log "El release $TAG ya existe."
        RELEASE_READY=1
        return 0
    fi
    if [[ $ASSUME_YES -eq 0 ]]; then
        local ans
        read -r -p "¿Crear el release $TAG en GitHub? [y/N] " ans
        [[ "$ans" =~ ^[Yy]$ ]] || { warn "publicación cancelada"; return 1; }
    fi
    log "Creando release $TAG…"
    gh release create "$TAG" --target "$(git rev-parse HEAD)" \
        --title "Rustidian $TAG" --generate-notes || return 1
    RELEASE_READY=1
    return 0
}

publish() {
    local files=()
    local f
    for f in ${PRODUCED[@]+"${PRODUCED[@]}"}; do
        [[ -f "$f" ]] && files+=("$f")
    done

    if [[ ${#files[@]} -gt 0 ]]; then
        printf '\nArtefactos a publicar en %s%s%s:\n' "$C_BOLD" "$TAG" "$C_RESET"
        printf '  %s\n' "${files[@]}"
    fi

    ensure_release || return 1

    if [[ ${#files[@]} -eq 0 ]]; then
        warn "no hay artefactos locales que subir"
        return 0
    fi

    gh release upload "$TAG" "${files[@]}" --clobber || return 1
    ok "Release: $(gh release view "$TAG" --json url --jq .url 2>/dev/null || echo "$TAG")"
}

run_target() {
    local label="$1" fn="$2"
    shift 2
    log "Objetivo: $label"
    if "$fn" "$@"; then
        ok "$label completado"
    else
        warn "$label falló"
        FAILED+=("$label")
    fi
}

# ── Main ─────────────────────────────────────────────────────────────────────
printf '%sRustidian — generador de binarios%s\n' "$C_BOLD" "$C_RESET"
printf '  versión : %s\n  tag     : %s\n  salida  : %s\n\n' "$VERSION" "$TAG" "$OUT_DIR"

# La compilación base del host reutiliza el binario para .deb y .rpm.
need_build=0
for flag in DO_DEB DO_RPM; do [[ ${!flag} -eq 1 ]] && need_build=1; done
if [[ $need_build -eq 1 ]]; then
    build_host || die "falló la compilación base"
fi

[[ $DO_DEB -eq 1 ]] && run_target "Debian (.deb)"  build_deb
[[ $DO_RPM -eq 1 ]] && run_target "Fedora (.rpm)"  build_rpm

# Objetivos que no se pueden construir en local se agregan a CI_PLATFORMS.
CI_PLATFORMS=""
if [[ $DO_WINDOWS -eq 1 ]]; then
    if [[ $WINDOWS_CI -eq 1 ]]; then
        CI_PLATFORMS="${CI_PLATFORMS}windows,"
    elif windows_can_build_local; then
        run_target "Windows (.zip)" build_windows
    else
        warn "no se puede cross-compilar Windows en local (falta mingw-w64 o el target Rust)"
        if [[ $PUBLISH -eq 1 ]] && have gh; then
            warn "se compilará Windows en GitHub Actions"
            CI_PLATFORMS="${CI_PLATFORMS}windows,"
        else
            err "para compilarlo aquí instala mingw-w64 y rustup + 'rustup target add x86_64-pc-windows-gnu'"
            FAILED+=("Windows (.zip)")
        fi
    fi
fi

if [[ $DO_MACOS -eq 1 ]]; then
    if [[ $MACOS_CI -eq 1 ]]; then
        CI_PLATFORMS="${CI_PLATFORMS}macos,"
    elif [[ "$HOST_OS" == "Darwin" ]]; then
        run_target "macOS (.dmg)" build_macos_local
    else
        CI_PLATFORMS="${CI_PLATFORMS}macos,"
    fi
fi

if [[ $PUBLISH -eq 1 ]]; then
    publish
fi

if [[ -n "$CI_PLATFORMS" ]]; then
    if [[ $PUBLISH -eq 1 ]]; then
        run_target "CI (${CI_PLATFORMS%,})" dispatch_ci "${CI_PLATFORMS%,}"
    else
        warn "se pidió --no-release: no se puede compilar en CI (los artefactos se suben a un release)"
        FAILED+=("CI (${CI_PLATFORMS%,})")
    fi
fi

printf '\n'
if [[ ${#FAILED[@]} -gt 0 ]]; then
    err "objetivos con error: ${FAILED[*]}"
fi

printf '\n%sArtefactos en %s:%s\n' "$C_BOLD" "$OUT_DIR" "$C_RESET"
find "$OUT_DIR" -maxdepth 1 -type f -printf '  %f\n' 2>/dev/null | sort

if [[ ${#FAILED[@]} -gt 0 ]]; then
    exit 1
fi
