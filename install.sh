#!/bin/sh
#
# Rustidian — instalador para Linux (y macOS como binario suelto).
#
# Instalación por usuario (sin sudo, recomendada):
#   curl -fsSL https://raw.githubusercontent.com/Hu-Tao128/rustidian/main/install.sh | sh
#
# Instalación en el sistema (/opt + /usr/local, requiere sudo):
#   curl -fsSL https://raw.githubusercontent.com/Hu-Tao128/rustidian/main/install.sh | sudo sh -s -- --system
#
# Opciones:
#   --version <v>   Instala una versión concreta (por defecto: la última).
#   --system        Instala en /opt/rustidian y /usr/local (requiere root).
#   --update        Actualiza a la última versión (idempotente).
#   --uninstall     Desinstala (conserva la configuración salvo --purge).
#   --purge         Con --uninstall, borra también ~/.config/rustidian.
#   --no-desktop    No instala el lanzador .desktop ni el icono.
#   --dry-run       Muestra lo que haría sin cambiar nada.
#   -h, --help      Muestra esta ayuda.
#
set -eu

REPO="Hu-Tao128/rustidian"
BIN_NAME="rustidian-ui"
APP_NAME="Rustidian"

SYSTEM=0
UNINSTALL=0
PURGE=0
DESKTOP=1
DRY_RUN=0
VERSION=""

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }

usage() {
    sed -n '2,20p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//' || true
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:-}"; [ -n "$VERSION" ] || die "--version requiere un valor"; shift ;;
        --system) SYSTEM=1 ;;
        --update) : ;;
        --uninstall) UNINSTALL=1 ;;
        --purge) PURGE=1 ;;
        --no-desktop) DESKTOP=0 ;;
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) die "opción desconocida: $1 (usa --help)" ;;
    esac
    shift
done

# ── Plataforma ───────────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS-$ARCH" in
    Linux-x86_64)                TARGET="x86_64-unknown-linux-gnu" ;;
    Linux-aarch64|Linux-arm64)   TARGET="aarch64-unknown-linux-gnu" ;;
    Darwin-x86_64)               TARGET="x86_64-apple-darwin" ;;
    Darwin-arm64|Darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
    *) die "plataforma no soportada: $OS-$ARCH" ;;
esac

# ── Rutas según el modo ──────────────────────────────────────────────────────
if [ "$SYSTEM" -eq 1 ]; then
    [ "$(id -u)" -eq 0 ] || die "el modo --system requiere root; usa: ... | sudo sh -s -- --system"
    BIN_DIR="/opt/rustidian"
    LINK_DIR="/usr/local/bin"
    DESKTOP_DIR="/usr/share/applications"
    ICON_DIR="/usr/share/icons/hicolor/scalable/apps"
else
    BIN_DIR="${HOME:?}/.local/bin"
    LINK_DIR=""
    DESKTOP_DIR="${HOME}/.local/share/applications"
    ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
fi
BIN_PATH="$BIN_DIR/$BIN_NAME"
LINK_PATH=""
[ -n "$LINK_DIR" ] && LINK_PATH="$LINK_DIR/$BIN_NAME"
DESKTOP_PATH="$DESKTOP_DIR/rustidian.desktop"
ICON_PATH="$ICON_DIR/rustidian.svg"
CONFIG_DIR="${HOME}/.config/rustidian"

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  [dry-run] %s\n' "$*"
    else
        "$@"
    fi
}

# ── Desinstalación ───────────────────────────────────────────────────────────
if [ "$UNINSTALL" -eq 1 ]; then
    info "Desinstalando $APP_NAME ($BIN_DIR)…"
    run rm -f "$BIN_PATH" "$DESKTOP_PATH" "$ICON_PATH"
    [ -n "$LINK_PATH" ] && run rm -f "$LINK_PATH"
    if [ "$PURGE" -eq 1 ]; then
        info "Borrando configuración ($CONFIG_DIR)…"
        run rm -rf "$CONFIG_DIR"
    fi
    info "Hecho. (La configuración se conserva en $CONFIG_DIR salvo --purge)"
    exit 0
fi

# ── Herramientas de descarga ─────────────────────────────────────────────────
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    download() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    download() { wget -qO "$2" "$1"; }
else
    die "se necesita 'curl' o 'wget'"
fi

# ── Resolver versión/tag ─────────────────────────────────────────────────────
if [ -z "$VERSION" ]; then
    info "Buscando la última versión de $APP_NAME…"
    TAG="$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
        | grep -o '"tag_name":[^,]*' | head -1 | cut -d'"' -f4)"
    [ -n "$TAG" ] || die "no se pudo determinar la última versión"
else
    case "$VERSION" in
        v*) TAG="$VERSION" ;;
        *)  TAG="v$VERSION" ;;
    esac
fi
info "Versión: $TAG ($TARGET)"

BASE_URL="https://github.com/$REPO/releases/download/$TAG"
BIN_URL="$BASE_URL/$BIN_NAME-$TARGET"
SUM_URL="$BIN_URL.sha256"
DESKTOP_URL="$BASE_URL/rustidian.desktop"
SVG_URL="$BASE_URL/rustidian.svg"
SVG_WHITE_URL="$BASE_URL/rustidian-white.svg"

# ── Descarga y verificación ──────────────────────────────────────────────────
TMP="$(mktemp -d 2>/dev/null || mktemp -d -t rustidian)"
trap 'rm -rf "$TMP"' EXIT INT TERM

info "Descargando $BIN_NAME-$TARGET…"
download "$BIN_URL" "$TMP/$BIN_NAME" || die "no se pudo descargar $BIN_URL"

if download "$SUM_URL" "$TMP/$BIN_NAME.sha256" 2>/dev/null && command -v sha256sum >/dev/null 2>&1; then
    expected="$(cut -d' ' -f1 < "$TMP/$BIN_NAME.sha256")"
    actual="$(sha256sum "$TMP/$BIN_NAME" | cut -d' ' -f1)"
    [ "$expected" = "$actual" ] || die "checksum inválido (esperado $expected, obtenido $actual)"
    info "Checksum verificado."
elif command -v shasum >/dev/null 2>&1 && [ -f "$TMP/$BIN_NAME.sha256" ]; then
    expected="$(cut -d' ' -f1 < "$TMP/$BIN_NAME.sha256")"
    actual="$(shasum -a 256 "$TMP/$BIN_NAME" | cut -d' ' -f1)"
    [ "$expected" = "$actual" ] || die "checksum inválido"
    info "Checksum verificado."
else
    warn "no se pudo verificar el checksum"
fi
chmod +x "$TMP/$BIN_NAME"

# ── Instalación del binario ──────────────────────────────────────────────────
info "Instalando en $BIN_PATH…"
run mkdir -p "$BIN_DIR"
run cp "$TMP/$BIN_NAME" "$BIN_PATH"
run chmod 755 "$BIN_PATH"
if [ -n "$LINK_PATH" ]; then
    run mkdir -p "$LINK_DIR"
    run ln -sf "$BIN_PATH" "$LINK_PATH"
fi

# ── Lanzador de escritorio + icono ───────────────────────────────────────────
if [ "$DESKTOP" -eq 1 ]; then
    info "Instalando lanzador e icono…"
    run mkdir -p "$DESKTOP_DIR" "$ICON_DIR"
    if download "$DESKTOP_URL" "$TMP/rustidian.desktop" 2>/dev/null; then
        run cp "$TMP/rustidian.desktop" "$DESKTOP_PATH"
    else
        if [ "$DRY_RUN" -eq 1 ]; then
            printf '  [dry-run] escribir %s\n' "$DESKTOP_PATH"
        else
            cat > "$DESKTOP_PATH" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=$APP_NAME
GenericName=Markdown Note Editor
Comment=Ultralight Markdown note editor built with Rust and Slint
Exec=$BIN_NAME
Icon=rustidian
Terminal=false
Categories=Office;TextEditor;Utility;
Keywords=markdown;notes;notas;editor;rustidian;
StartupNotify=true
EOF
        fi
    fi
    download "$SVG_URL" "$TMP/rustidian.svg" 2>/dev/null \
        && run cp "$TMP/rustidian.svg" "$ICON_PATH" \
        || warn "no se pudo descargar el icono"
    download "$SVG_WHITE_URL" "$TMP/rustidian-white.svg" 2>/dev/null \
        && run cp "$TMP/rustidian-white.svg" "$ICON_DIR/rustidian-white.svg" \
        || true
    command -v update-desktop-database >/dev/null 2>&1 \
        && run update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

# ── Aviso de PATH ────────────────────────────────────────────────────────────
if [ "$SYSTEM" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *)
            printf '\n%s está en %s, que no está en tu PATH. Añádelo:\n' "$BIN_NAME" "$BIN_DIR"
            printf '  echo '\''export PATH="%s:$PATH"'\'' >> ~/.profile\n' "$BIN_DIR"
            ;;
    esac
fi

info "$APP_NAME $TAG instalado."
[ "$SYSTEM" -eq 0 ] && info "Ejecútalo con: $BIN_NAME   (o desde tu menú de aplicaciones)"
