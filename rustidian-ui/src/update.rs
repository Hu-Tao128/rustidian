//! Auto-actualización desde GitHub Releases.
//!
//! El release publica un binario crudo por target (`rustidian-ui-<triple>`) y su
//! `.sha256`; este módulo consulta la última release, compara versiones,
//! descarga el binario del target actual, verifica el checksum y reemplaza el
//! ejecutable en caliente con `self-replace`.
//!
//! No usa Slint: la UI lanza las funciones en un hilo y vuelve al event loop.

use std::io::Read;
use std::path::PathBuf;

const API_LATEST: &str = "https://api.github.com/repos/Hu-Tao128/rustidian/releases/latest";
const USER_AGENT: &str = concat!("rustidian/", env!("CARGO_PKG_VERSION"));

/// Datos de una actualización disponible.
pub struct UpdateInfo {
    /// Versión sin la `v` inicial (p. ej. `0.9.0`).
    pub version: String,
    pub download_url: String,
    pub checksum_url: Option<String>,
}

fn target() -> &'static str {
    env!("RUSTIDIAN_TARGET")
}

/// Nombre del asset del binario para el target actual.
fn asset_name() -> String {
    let t = target();
    if t.contains("windows") {
        format!("rustidian-ui-{t}.exe")
    } else {
        format!("rustidian-ui-{t}")
    }
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .set(
            "Accept",
            "application/octet-stream, application/vnd.github+json",
        )
        .call()
        .map_err(|e| format!("error de red: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("error de lectura: {e}"))?;
    Ok(buf)
}

fn get_string(url: &str) -> Result<String, String> {
    String::from_utf8(get(url)?).map_err(|e| format!("respuesta no UTF-8: {e}"))
}

fn version_parts(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// `true` si `remote` es una versión estrictamente mayor que `current`.
pub fn is_newer(remote: &str, current: &str) -> bool {
    let r = version_parts(remote);
    let c = version_parts(current);
    for i in 0..r.len().max(c.len()) {
        let a = r.get(i).copied().unwrap_or(0);
        let b = c.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// Consulta la última release. `Ok(None)` = ya está al día.
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let body = get_string(API_LATEST)?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON inválido: {e}"))?;

    let tag = json["tag_name"]
        .as_str()
        .ok_or("la respuesta no contiene tag_name")?
        .to_string();
    let current = env!("CARGO_PKG_VERSION");
    if !is_newer(&tag, current) {
        return Ok(None);
    }

    let want = asset_name();
    let want_sum = format!("{want}.sha256");
    let assets = json["assets"]
        .as_array()
        .ok_or("la respuesta no contiene assets")?;

    let mut download_url = None;
    let mut checksum_url = None;
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        let url = asset["browser_download_url"].as_str().unwrap_or("");
        if name == want {
            download_url = Some(url.to_string());
        } else if name == want_sum {
            checksum_url = Some(url.to_string());
        }
    }

    let download_url = download_url.ok_or_else(|| format!("el release {tag} no incluye {want}"))?;
    Ok(Some(UpdateInfo {
        version: tag.trim_start_matches('v').to_string(),
        download_url,
        checksum_url,
    }))
}

/// Descarga, verifica y reemplaza el ejecutable actual.
pub fn download_and_apply(info: &UpdateInfo) -> Result<(), String> {
    let bytes = get(&info.download_url)?;

    if let Some(sum_url) = &info.checksum_url {
        let text = get_string(sum_url)?;
        let expected = text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if !expected.is_empty() {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&bytes);
            let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            if actual != expected {
                return Err(format!(
                    "checksum inválido (esperado {expected}, obtenido {actual})"
                ));
            }
        }
    }

    let exe =
        std::env::current_exe().map_err(|e| format!("no se pudo localizar el ejecutable: {e}"))?;
    let dir: PathBuf = exe
        .parent()
        .ok_or("ruta de ejecutable inválida")?
        .to_path_buf();
    let tmp = dir.join(".rustidian-ui.update");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| format!("no se pudo escribir la actualización (¿permisos?): {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }

    self_replace::self_replace(&tmp)
        .map_err(|e| format!("no se pudo reemplazar el binario: {e}"))?;
    Ok(())
}

/// Gestiona los argumentos de línea de comandos. Devuelve `Some(código)` si el
/// programa debe terminar sin abrir la GUI.
pub fn handle_cli(args: &[String]) -> Option<i32> {
    let has = |flag: &str| args.iter().any(|a| a == flag);

    if has("--version") || has("-V") {
        println!("Rustidian {}", env!("CARGO_PKG_VERSION"));
        return Some(0);
    }
    if has("--help") || has("-h") {
        print_help();
        return Some(0);
    }
    if has("--check") {
        return Some(cli_check());
    }
    if has("--update") {
        return Some(cli_update());
    }
    None
}

fn cli_check() -> i32 {
    match check() {
        Ok(Some(info)) => {
            println!(
                "Nueva versión disponible: {} (actual {}).",
                info.version,
                env!("CARGO_PKG_VERSION")
            );
            0
        }
        Ok(None) => {
            println!(
                "Ya tienes la última versión ({}).",
                env!("CARGO_PKG_VERSION")
            );
            0
        }
        Err(e) => {
            eprintln!("No se pudieron comprobar las actualizaciones: {e}");
            1
        }
    }
}

fn cli_update() -> i32 {
    match check() {
        Ok(None) => {
            println!(
                "Ya tienes la última versión ({}).",
                env!("CARGO_PKG_VERSION")
            );
            0
        }
        Ok(Some(info)) => {
            println!("Actualizando a {}…", info.version);
            match download_and_apply(&info) {
                Ok(()) => {
                    println!("Actualizado. Reinicia Rustidian para usar la nueva versión.");
                    0
                }
                Err(e) => {
                    eprintln!("Error al actualizar: {e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("No se pudieron comprobar las actualizaciones: {e}");
            1
        }
    }
}

fn print_help() {
    println!(
        "Rustidian {}\n\
         \n\
         Uso:\n\
         \x20 rustidian-ui            Abre el editor.\n\
         \x20 rustidian-ui --check    Comprueba si hay una versión nueva.\n\
         \x20 rustidian-ui --update   Descarga e instala la última versión.\n\
         \x20 rustidian-ui --version  Muestra la versión.\n\
         \x20 rustidian-ui --help     Muestra esta ayuda.",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_newer_versions() {
        assert!(is_newer("v0.9.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(is_newer("v0.2.0-beta.1", "0.1.0"));
    }
}
